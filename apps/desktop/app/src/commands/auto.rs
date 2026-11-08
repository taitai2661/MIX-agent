//! Auto model resolution: the DB-backed half of `mixd_core::auto`.
//!
//! Candidates are the stored models opted into Auto (`data.auto`), minus rows
//! whose provider no longer exists. Feedback (`message_feedback`) and failure
//! history (`auto_reliability_events`) feed the scorer; both tables already
//! exist in the schema but were previously write-only. A fresh install with no
//! history still routes sensibly on capabilities + quality priors alone.

use std::collections::HashMap;

use mixd_core::auto::{
    AutoCandidate, AutoPriority, AutoSelectOptions, AutoSelection, CandidateSignals,
};
use mixd_core::runs::engine::TurnPerformance;
use serde::Serialize;

/// What the UI / engine needs after an Auto turn is resolved.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedAuto {
    pub model_id: String,
    pub provider_id: String,
    pub context_window: u32,
    pub selection: AutoSelection,
}

type CandidateRow = (
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Every Auto candidate whose provider still exists, newest first.
pub async fn fetch_auto_candidates(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Vec<AutoCandidate>, String> {
    let rows: Vec<CandidateRow> = sqlx::query_as(
        "SELECT m.id, COALESCE(json_extract(m.data, '$.providerId'), ''), \
          COALESCE(json_extract(m.data, '$.label'), ''), \
          COALESCE(json_extract(m.data, '$.contextWindow'), 0), \
          COALESCE(json_extract(m.data, '$.vision'), 0), \
          COALESCE(json_extract(m.data, '$.toolCalling'), 0), \
          COALESCE(json_extract(m.data, '$.reasoning'), 0), \
          json_extract(p.data, '$.kind'), \
          json_extract(p.data, '$.preset'), \
          json_extract(p.data, '$.baseUrl') \
          FROM models m JOIN providers p ON p.id = json_extract(m.data, '$.providerId') \
          WHERE COALESCE(json_extract(m.data, '$.auto'), 0) != 0 \
          ORDER BY m.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                provider_id,
                label,
                context_window,
                vision,
                tool_calling,
                reasoning,
                kind,
                preset,
                base_url,
            )| {
                // The stored id is `<provider_id>:<model>`; the wire name is the
                // part after the first colon (Ollama tags keep their own colon).
                let model_name = id
                    .split_once(':')
                    .map(|(_, rest)| rest)
                    .filter(|rest| !rest.is_empty())
                    .unwrap_or(&label)
                    .to_string();
                AutoCandidate {
                    id,
                    provider_id,
                    model_name,
                    context_window: context_window.max(0) as u32,
                    vision: vision != 0,
                    tool_calling: tool_calling != 0,
                    reasoning: reasoning != 0,
                    is_local: crate::commands::providers::is_local_provider(
                        kind.as_deref(),
                        preset.as_deref(),
                        base_url.as_deref(),
                    ),
                }
            },
        )
        .collect())
}

/// Rough input-token estimate for routing: ~4 chars per token over the turn
/// plus a per-message overhead. Deliberately conservative (over-estimates).
///
/// `attachment_tokens` is supplied by the caller because image/PDF payloads
/// are not represented by their base64 character count in the prompt: an
/// image is billed as a fixed tile budget, not as megabytes of text.
pub fn estimate_required_tokens(
    parts: &[String],
    reserved_output_tokens: u64,
    attachment_tokens: u64,
) -> u64 {
    let chars: usize = parts.iter().map(|p| p.len()).sum();
    (chars as u64 / 4) + (parts.len() as u64 * 8) + reserved_output_tokens + attachment_tokens
}

/// Explicit feedback weighted by age for one route profile.
#[derive(Default)]
struct FeedbackSummary {
    /// `(up, down)` for the profile being routed.
    by_profile: HashMap<String, (f64, f64)>,
    /// Weighted total across every profile, for the UCB exploration term.
    total: HashMap<String, f64>,
}

/// Recent feedback, decayed with a 60-day half-life.
///
/// The old query read every rating ever written and filtered profiles in Rust.
/// It is now bounded to 180 days (backed by an index) and weighted so last
/// week's rating counts more than last year's.
async fn feedback_summary(pool: &sqlx::Pool<sqlx::Sqlite>, profile: &str) -> FeedbackSummary {
    type Row = (Option<String>, Option<String>, Option<String>, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT json_extract(data, '$.modelId'), json_extract(data, '$.profile'), \
                json_extract(data, '$.value'), created_at \
           FROM message_feedback \
          WHERE owner_id = ? AND created_at >= datetime('now', '-180 days')",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let now = chrono::Utc::now();
    let mut out = FeedbackSummary::default();
    for (model_id, row_profile, value, created_at) in rows {
        let (Some(model_id), Some(value)) = (model_id, value) else {
            continue;
        };
        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&created_at) else {
            continue;
        };
        let age_days = now
            .signed_duration_since(ts.with_timezone(&chrono::Utc))
            .num_seconds() as f64
            / 86_400.0;
        let weight = 0.5f64.powf(age_days.max(0.0) / 60.0);
        if !matches!(value.as_str(), "up" | "down") {
            continue;
        }
        *out.total.entry(model_id.clone()).or_default() += weight;
        if row_profile.as_deref() != Some(profile) {
            continue;
        }
        let entry = out.by_profile.entry(model_id).or_default();
        match value.as_str() {
            "up" => entry.0 += weight,
            "down" => entry.1 += weight,
            _ => unreachable!("value was checked above"),
        }
    }
    out
}

#[derive(Default)]
struct ReliabilitySignals {
    /// Penalty and reason keyed by model record id.
    by_model: HashMap<String, (f64, Option<String>)>,
    /// Last authentication failure per provider. Applied to every candidate of
    /// that provider in `resolve_auto_model`, where the model/provider mapping
    /// is available.
    auth_providers: HashMap<String, chrono::DateTime<chrono::Utc>>,
}

/// Recent failures put a model (or its provider) in a short cooldown.
///
/// Mirrors Docker's windows: 2+ model failures in 10 min, or 3+ distinct
/// models of one provider failing in 5 min. Authentication failures are a
/// provider-level problem, not a model-quality signal: they cool the provider
/// down for 30 minutes so a broken key does not permanently starve Auto.
async fn reliability_penalties(pool: &sqlx::Pool<sqlx::Sqlite>) -> ReliabilitySignals {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT data, created_at FROM auto_reliability_events \
          WHERE owner_id = ? AND created_at >= datetime('now', '-30 days')",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let now = chrono::Utc::now();
    let mut model_failures: HashMap<String, Vec<chrono::DateTime<chrono::Utc>>> = HashMap::new();
    let mut provider_models: HashMap<String, HashMap<String, Vec<chrono::DateTime<chrono::Utc>>>> =
        HashMap::new();
    let mut provider_auth: HashMap<String, Vec<chrono::DateTime<chrono::Utc>>> = HashMap::new();
    for (raw, created_at) in rows {
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        if value.get("outcome").and_then(|v| v.as_str()) != Some("failure") {
            continue;
        }
        let classification = value
            .get("classification")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&created_at) else {
            continue;
        };
        let ts = ts.with_timezone(&chrono::Utc);
        let provider_id = value.get("providerId").and_then(|v| v.as_str());
        if classification == "auth" {
            if let Some(provider_id) = provider_id {
                provider_auth
                    .entry(provider_id.to_string())
                    .or_default()
                    .push(ts);
            }
            continue;
        }
        // `incomplete` is the legacy name for a stream that ended early;
        // `transport` is a connection failure that survived retries.
        if !matches!(
            classification,
            "rate_limit" | "provider_5xx" | "timeout" | "transport" | "incomplete"
        ) {
            continue;
        }
        let Some(model_id) = value.get("modelId").and_then(|v| v.as_str()) else {
            continue;
        };
        model_failures
            .entry(model_id.to_string())
            .or_default()
            .push(ts);
        if let Some(provider_id) = provider_id {
            provider_models
                .entry(provider_id.to_string())
                .or_default()
                .entry(model_id.to_string())
                .or_default()
                .push(ts);
        }
    }
    let recent = |times: &[chrono::DateTime<chrono::Utc>], minutes: i64| {
        times
            .iter()
            .filter(|t| now.signed_duration_since(**t).num_minutes() < minutes)
            .count()
    };
    let mut out = ReliabilitySignals::default();
    for (model_id, times) in &model_failures {
        if recent(times, 10) >= 2 {
            out.by_model.insert(
                model_id.clone(),
                (
                    0.70,
                    Some("最近このモデルで失敗が続いたため一時回避".to_string()),
                ),
            );
        }
    }
    for (provider_id, models) in &provider_models {
        let distinct_recent = models.values().filter(|times| recent(times, 5) > 0).count();
        if distinct_recent >= 3 {
            for model_id in models.keys() {
                out.by_model.entry(model_id.clone()).or_insert((
                    0.70,
                    Some(format!(
                        "プロバイダ {provider_id} が一時的に不安定（クールダウン中）"
                    )),
                ));
            }
        }
    }
    for (provider_id, times) in &provider_auth {
        if recent(times, 30) == 0 {
            continue;
        }
        if let Some(latest) = times
            .iter()
            .filter(|t| now.signed_duration_since(**t).num_minutes() < 30)
            .max()
        {
            out.auth_providers.insert(provider_id.clone(), *latest);
        }
    }
    out
}

/// One model's measured speed, from `auto_model_stats`.
#[derive(Debug, Clone, Copy, Default)]
struct SpeedSample {
    ttft_ms: Option<f64>,
    output_tps: Option<f64>,
}

/// Read measured speed for the current profile, falling back to the `'*'`
/// aggregate while a profile-specific row is still cold.
async fn measured_speed_samples(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    profile: &str,
) -> HashMap<String, SpeedSample> {
    type Row = (String, String, Option<f64>, Option<f64>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT model_id, profile, ewma_ttft_ms, ewma_output_tps \
           FROM auto_model_stats \
          WHERE profile IN (?, '*') AND updated_at >= datetime('now', '-30 days')",
    )
    .bind(profile)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut out: HashMap<String, SpeedSample> = HashMap::new();
    let mut exact: HashMap<String, bool> = HashMap::new();
    for (model_id, row_profile, ttft_ms, output_tps) in rows {
        let is_exact = row_profile == profile;
        if exact.get(&model_id).copied().unwrap_or(false) && !is_exact {
            continue;
        }
        if is_exact {
            exact.insert(model_id.clone(), true);
        }
        out.insert(
            model_id,
            SpeedSample {
                ttft_ms,
                output_tps,
            },
        );
    }
    out
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.retain(|value| value.is_finite() && *value > 0.0);
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[values.len() / 2])
}

/// Relative speed in `[-0.16, 0.16]` (the scorer's established contract).
///
/// Absolute milliseconds are not comparable across providers (a local 8B and
/// a frontier cloud model have different physics), so each side is scored
/// against the candidate median. Models without measurements get no bonus and
/// no penalty; the router never invents speed it has not observed.
fn relative_speed_scores(
    candidates: &[AutoCandidate],
    samples: &HashMap<String, SpeedSample>,
) -> HashMap<String, f64> {
    let ttfts = candidates
        .iter()
        .filter_map(|c| samples.get(&c.id).and_then(|s| s.ttft_ms))
        .collect::<Vec<_>>();
    let throughputs = candidates
        .iter()
        .filter_map(|c| samples.get(&c.id).and_then(|s| s.output_tps))
        .collect::<Vec<_>>();
    let ttft_median = median(ttfts);
    let tps_median = median(throughputs);
    let mut out = HashMap::new();
    for c in candidates {
        let Some(sample) = samples.get(&c.id) else {
            continue;
        };
        let mut sum = 0.0;
        let mut parts = 0.0;
        if let (Some(ttft), Some(median)) = (sample.ttft_ms, ttft_median) {
            if median > 0.0 {
                sum += ((median - ttft) / median).clamp(-1.0, 1.0);
                parts += 1.0;
            }
        }
        if let (Some(tps), Some(median)) = (sample.output_tps, tps_median) {
            if median > 0.0 {
                sum += ((tps - median) / median).clamp(-1.0, 1.0);
                parts += 1.0;
            }
        }
        if parts > 0.0 {
            out.insert(c.id.clone(), (sum / parts) * 0.16);
        }
    }
    out
}

/// Resolve `auto` to a concrete model for one turn.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_auto_model(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    content: &str,
    mode: &str,
    tools_required: bool,
    has_image: bool,
    context_parts: &[String],
    attachment_tokens: u64,
    reserved_output_tokens: u64,
    request_key: &str,
    current_model_id: Option<&str>,
) -> Result<ResolvedAuto, String> {
    resolve_auto_model_excluding(
        pool,
        content,
        mode,
        tools_required,
        has_image,
        context_parts,
        attachment_tokens,
        reserved_output_tokens,
        request_key,
        current_model_id,
        &[],
        None,
    )
    .await
}

/// Resolve an Auto fallback while excluding models that already failed.
///
/// `prefer_other_provider_than` gives candidates from a different provider a
/// scoring advantage when one exists. The request key is derived by the caller
/// so a retry stays deterministic.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_auto_model_excluding(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    content: &str,
    mode: &str,
    tools_required: bool,
    has_image: bool,
    context_parts: &[String],
    attachment_tokens: u64,
    reserved_output_tokens: u64,
    request_key: &str,
    current_model_id: Option<&str>,
    excluded_ids: &[String],
    prefer_other_provider_than: Option<&str>,
) -> Result<ResolvedAuto, String> {
    let candidates = fetch_auto_candidates(pool).await?;
    if candidates.is_empty() {
        return Err(
            "Autoで使用可能なモデルがありません。設定 → モデル でAuto候補を有効にしてください。"
                .to_string(),
        );
    }
    let required_tokens =
        estimate_required_tokens(context_parts, reserved_output_tokens, attachment_tokens);
    let profile = mixd_core::auto::routing_profile(content, mode, tools_required, has_image);
    let feedback = feedback_summary(pool, &profile).await;
    let penalties = reliability_penalties(pool).await;
    let speed = measured_speed_samples(pool, &profile).await;
    let speed_scores = relative_speed_scores(&candidates, &speed);
    let settings = crate::commands::settings::read_settings(pool).await.ok();
    let priority = settings
        .as_ref()
        .map(|settings| AutoPriority::parse(&settings.auto_priority))
        .unwrap_or_default();
    let mut signals: HashMap<String, CandidateSignals> = HashMap::new();
    for c in &candidates {
        let fb = feedback
            .by_profile
            .get(&c.id)
            .copied()
            .unwrap_or((0.0, 0.0));
        let mut penalty = penalties
            .by_model
            .get(&c.id)
            .cloned()
            .unwrap_or((0.0, None));
        if penalty.0 == 0.0 && penalties.auth_providers.contains_key(&c.provider_id) {
            penalty = (
                0.50,
                Some(format!(
                    "プロバイダ {} の認証に失敗（設定を確認してください）",
                    c.provider_id
                )),
            );
        }
        signals.insert(
            c.id.clone(),
            CandidateSignals {
                feedback: fb,
                _total_feedback: feedback.total.get(&c.id).copied().unwrap_or(0.0),
                reliability_penalty: penalty.0,
                reliability_reason: penalty.1,
                speed_score: speed_scores.get(&c.id).copied().unwrap_or(0.0),
            },
        );
    }
    let prefer_local = settings
        .as_ref()
        .map(|settings| settings.local_prefer)
        .unwrap_or(false);
    let opts = AutoSelectOptions {
        content: content.to_string(),
        mode: mode.to_string(),
        tools_required,
        has_image,
        required_tokens,
        current_model_id: current_model_id.map(|s| s.to_string()),
        prefer_other_provider_than: prefer_other_provider_than.map(|s| s.to_string()),
        priority,
        request_key: request_key.to_string(),
        prefer_local,
    };
    let (selected, selection) =
        mixd_core::auto::select_auto_model(&candidates, &signals, &opts, excluded_ids)?;
    Ok(ResolvedAuto {
        model_id: selected.id.clone(),
        provider_id: selected.provider_id.clone(),
        context_window: selected.context_window,
        selection,
    })
}

/// Record one Auto turn outcome for future routing.
#[allow(clippy::too_many_arguments)]
pub async fn record_auto_outcome(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    model_id: &str,
    provider_id: &str,
    scope: &str,
    outcome: &str,
    classification: Option<&str>,
    profile: Option<&str>,
    required_tokens: Option<u64>,
) {
    let data = serde_json::json!({
        "modelId": model_id,
        "providerId": provider_id,
        "scope": scope,
        "outcome": outcome,
        "classification": classification,
        "profile": profile,
        "requiredTokens": required_tokens,
    });
    let _ =
        sqlx::query("INSERT INTO auto_reliability_events (id, owner_id, data) VALUES (?, ?, ?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(data.to_string())
            .execute(pool)
            .await;
    // Bounded retention: keep the table small without a scheduler.
    let _ = sqlx::query(
        "DELETE FROM auto_reliability_events WHERE created_at < datetime('now', '-30 days')",
    )
    .execute(pool)
    .await;
}

/// Persist one measured provider turn.
///
/// Called after the Run from the app layer, so writing statistics never sits
/// on a streaming turn's critical path. Both the profile-specific row and the
/// `'*'` aggregate are updated; reads prefer the exact profile and fall back to
/// the aggregate while it is cold.
pub async fn record_turn_performance(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    profile: &str,
    mode: &str,
    perf: &TurnPerformance,
) {
    let success = perf.failure_kind.is_none();
    let failure_kind = perf.failure_kind.map(|kind| kind.as_str());
    let (ttft, throughput, duration) = if success {
        let ttft = perf
            .ttft_ms
            .map(|value| (value as f64).clamp(0.0, 120_000.0));
        let duration = Some((perf.duration_ms as f64).clamp(0.0, 600_000.0));
        // Generation speed excludes the first-token wait: queueing time is a
        // property of the provider, not of how fast it emits once it starts.
        let throughput = match (perf.output_tokens, perf.duration_ms, perf.ttft_ms) {
            (Some(tokens), duration_ms, Some(ttft_ms)) if tokens > 0 && duration_ms > ttft_ms => {
                Some(
                    (tokens as f64 / ((duration_ms - ttft_ms) as f64 / 1000.0)).clamp(0.0, 1_000.0),
                )
            }
            _ => None,
        };
        (ttft, throughput, duration)
    } else {
        // A failed turn says nothing about speed; counting its (often very
        // short) duration as fast would reward models that fail quickly.
        (None, None, None)
    };
    upsert_auto_stat(
        pool,
        &perf.model_id,
        profile,
        success,
        ttft,
        throughput,
        duration,
        failure_kind,
    )
    .await;
    upsert_auto_stat(
        pool,
        &perf.model_id,
        "*",
        success,
        ttft,
        throughput,
        duration,
        failure_kind,
    )
    .await;
    if let Some(kind) = failure_kind {
        record_auto_outcome(
            pool,
            &perf.model_id,
            &perf.provider_id,
            mode,
            "failure",
            Some(kind),
            Some(profile),
            None,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn upsert_auto_stat(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    model_id: &str,
    profile: &str,
    success: bool,
    ttft_ms: Option<f64>,
    output_tps: Option<f64>,
    duration_ms: Option<f64>,
    failure_kind: Option<&str>,
) {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let last_failure_at = failure_kind.map(|_| now.clone());
    let _ = sqlx::query(
        "INSERT INTO auto_model_stats \
           (model_id, profile, success_count, failure_count, ewma_ttft_ms, \
            ewma_output_tps, ewma_duration_ms, last_failure_kind, last_failure_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(model_id, profile) DO UPDATE SET \
           success_count = success_count + excluded.success_count, \
           failure_count = failure_count + excluded.failure_count, \
           ewma_ttft_ms = CASE \
             WHEN excluded.ewma_ttft_ms IS NULL THEN ewma_ttft_ms \
             WHEN ewma_ttft_ms IS NULL THEN excluded.ewma_ttft_ms \
             ELSE ewma_ttft_ms * 0.8 + excluded.ewma_ttft_ms * 0.2 END, \
           ewma_output_tps = CASE \
             WHEN excluded.ewma_output_tps IS NULL THEN ewma_output_tps \
             WHEN ewma_output_tps IS NULL THEN excluded.ewma_output_tps \
             ELSE ewma_output_tps * 0.8 + excluded.ewma_output_tps * 0.2 END, \
           ewma_duration_ms = CASE \
             WHEN excluded.ewma_duration_ms IS NULL THEN ewma_duration_ms \
             WHEN ewma_duration_ms IS NULL THEN excluded.ewma_duration_ms \
             ELSE ewma_duration_ms * 0.8 + excluded.ewma_duration_ms * 0.2 END, \
           last_failure_kind = COALESCE(excluded.last_failure_kind, last_failure_kind), \
           last_failure_at = COALESCE(excluded.last_failure_at, last_failure_at), \
           updated_at = excluded.updated_at",
    )
    .bind(model_id)
    .bind(profile)
    .bind(if success { 1i64 } else { 0i64 })
    .bind(if success { 0i64 } else { 1i64 })
    .bind(ttft_ms)
    .bind(output_tps)
    .bind(duration_ms)
    .bind(failure_kind)
    .bind(last_failure_at)
    .bind(now)
    .execute(pool)
    .await;
}

#[tauri::command]
pub async fn resolve_auto(
    state: tauri::State<'_, crate::AppState>,
    content: String,
    mode: Option<String>,
    tools_required: Option<bool>,
    has_image: Option<bool>,
) -> Result<ResolvedAuto, String> {
    let mode = mode.unwrap_or_else(|| "chat".to_string());
    let parts = vec![content.clone()];
    resolve_auto_model(
        state.storage.pool(),
        &content,
        &mode,
        tools_required.unwrap_or(false),
        has_image.unwrap_or(false),
        &parts,
        0,
        4096,
        &format!("preview:{}", chrono::Utc::now().timestamp_millis()),
        None,
    )
    .await
}

/// Rate an Auto answer so future Auto turns learn from it.
///
/// Docker parity: only answers produced by an Auto turn (`requested_model_id
/// == "auto"`) can be rated. The rating is keyed to the assistant message and
/// stores the resolved model + route profile the scorer reads back.
#[tauri::command]
pub async fn set_message_feedback(
    state: tauri::State<'_, crate::AppState>,
    message_id: String,
    value: Option<String>,
) -> Result<Option<String>, String> {
    if let Some(v) = &value {
        if v != "up" && v != "down" {
            return Err("value must be \"up\", \"down\" or null".to_string());
        }
    }
    let pool = state.storage.pool();
    let row: Option<(String,)> = sqlx::query_as("SELECT data FROM messages WHERE id = ?")
        .bind(&message_id)
        .fetch_optional(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    let (raw,) = row.ok_or_else(|| "メッセージが見つかりません".to_string())?;
    let message: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "メッセージを読み込めませんでした".to_string())?;
    if message.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return Err("Autoで選択された回答のみ評価できます".to_string());
    }
    let selection = message.get("autoSelection").cloned();
    let Some(selection) = selection else {
        return Err("Autoで選択された回答のみ評価できます".to_string());
    };
    let model_record_id = selection
        .get("selected_id")
        .or_else(|| selection.get("model_record_id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let profile = selection
        .get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if model_record_id.is_empty() || profile.is_empty() {
        return Err("Autoで選択された回答のみ評価できます".to_string());
    }
    if value.is_none() {
        sqlx::query("DELETE FROM message_feedback WHERE message_id = ?")
            .bind(&message_id)
            .execute(pool)
            .await
            .map_err(crate::error::cmd_err)?;
        return Ok(None);
    }
    let value = value.unwrap();
    let data = serde_json::json!({
        "modelId": model_record_id,
        "profile": profile,
        "value": value,
    });
    sqlx::query(
        "INSERT INTO message_feedback (id, owner_id, data, message_id) VALUES (?, ?, ?, ?) \
         ON CONFLICT(message_id) DO UPDATE SET data = excluded.data",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(data.to_string())
    .bind(&message_id)
    .execute(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(Some(value))
}
