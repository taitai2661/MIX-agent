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
pub fn estimate_required_tokens(parts: &[String], reserved_output_tokens: u64) -> u64 {
    let chars: usize = parts.iter().map(|p| p.len()).sum();
    (chars as u64 / 4) + (parts.len() as u64 * 8) + reserved_output_tokens
}

async fn feedback_counts(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    profile: &str,
) -> HashMap<String, (f64, f64)> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT data FROM message_feedback")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let mut out: HashMap<String, (f64, f64)> = HashMap::new();
    for (raw,) in rows {
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        if value.get("profile").and_then(|v| v.as_str()) != Some(profile) {
            continue;
        }
        let Some(model_id) = value.get("modelId").and_then(|v| v.as_str()) else {
            continue;
        };
        let entry = out.entry(model_id.to_string()).or_default();
        match value.get("value").and_then(|v| v.as_str()) {
            Some("up") => entry.0 += 1.0,
            Some("down") => entry.1 += 1.0,
            _ => {}
        }
    }
    out
}

async fn reliability_penalties(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> HashMap<String, (f64, Option<String>)> {
    // Recent failures put a model (or its provider) in a short cooldown.
    // Mirrors Docker's windows: 2+ model failures in 10 min, or 3+ distinct
    // models of one provider failing in 5 min.
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT data, created_at FROM auto_reliability_events")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let now = chrono::Utc::now();
    let mut model_failures: HashMap<String, Vec<chrono::DateTime<chrono::Utc>>> = HashMap::new();
    let mut provider_models: HashMap<String, HashMap<String, Vec<chrono::DateTime<chrono::Utc>>>> =
        HashMap::new();
    for (raw, created_at) in rows {
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        if value.get("outcome").and_then(|v| v.as_str()) != Some("failure") {
            continue;
        }
        let classification = value
            .get("classification")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !matches!(
            classification,
            "rate_limit" | "provider_5xx" | "timeout" | "incomplete"
        ) {
            continue;
        }
        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&created_at) else {
            continue;
        };
        let ts = ts.with_timezone(&chrono::Utc);
        let Some(model_id) = value.get("modelId").and_then(|v| v.as_str()) else {
            continue;
        };
        model_failures
            .entry(model_id.to_string())
            .or_default()
            .push(ts);
        if let Some(provider_id) = value.get("providerId").and_then(|v| v.as_str()) {
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
    let mut out = HashMap::new();
    for (model_id, times) in &model_failures {
        if recent(times, 10) >= 2 {
            out.insert(
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
                out.entry(model_id.clone()).or_insert((
                    0.70,
                    Some(format!(
                        "プロバイダ {provider_id} が一時的に不安定（クールダウン中）"
                    )),
                ));
            }
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
    reserved_output_tokens: u64,
    request_key: &str,
    current_model_id: Option<&str>,
) -> Result<ResolvedAuto, String> {
    let candidates = fetch_auto_candidates(pool).await?;
    if candidates.is_empty() {
        return Err(
            "Autoで使用可能なモデルがありません。設定 → モデル でAuto候補を有効にしてください。"
                .to_string(),
        );
    }
    let required_tokens = estimate_required_tokens(context_parts, reserved_output_tokens);
    let profile = mixd_core::auto::routing_profile(content, mode, tools_required, has_image);
    let feedback = feedback_counts(pool, &profile).await;
    let penalties = reliability_penalties(pool).await;
    let mut signals: HashMap<String, CandidateSignals> = HashMap::new();
    for c in &candidates {
        let fb = feedback.get(&c.id).copied().unwrap_or((0.0, 0.0));
        let (penalty, reason) = penalties.get(&c.id).cloned().unwrap_or((0.0, None));
        signals.insert(
            c.id.clone(),
            CandidateSignals {
                feedback: fb,
                _total_feedback: 0.0,
                reliability_penalty: penalty,
                reliability_reason: reason,
                speed_score: 0.0,
            },
        );
    }
    let prefer_local = crate::commands::settings::read_settings(pool)
        .await
        .map(|settings| settings.local_prefer)
        .unwrap_or(false);
    let opts = AutoSelectOptions {
        content: content.to_string(),
        mode: mode.to_string(),
        tools_required,
        has_image,
        required_tokens,
        current_model_id: current_model_id.map(|s| s.to_string()),
        prefer_other_provider_than: None,
        priority: AutoPriority::Balanced,
        request_key: request_key.to_string(),
        prefer_local,
    };
    let excluded: Vec<String> = Vec::new();
    let (selected, selection) =
        mixd_core::auto::select_auto_model(&candidates, &signals, &opts, &excluded)?;
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
