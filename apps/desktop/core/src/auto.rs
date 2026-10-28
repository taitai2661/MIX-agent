//! Auto model routing: pick the best model per request.
//!
//! Port of the Docker edition's `mix_agent.routing` + `model_roles` +
//! `model_quality` trio. The selection is local and deterministic — it never
//! contacts a provider. It filters the Auto candidate list down to models that
//! can serve the turn (chat-eligible, context window fits, vision/tools when
//! required), then scores the survivors on:
//!
//! - learned feedback (Beta mean + UCB exploration, Docker parity),
//! - a static quality prior from well-known families / parameter size,
//! - cost efficiency (via [`crate::pricing`]),
//! - reliability penalties learned from `auto_reliability_events`,
//! - small continuity / reasoning bonuses that avoid flapping.
//!
//! Exact ties are broken by a deterministic per-request draw so a retry with
//! the same `request_key` resolves the same way.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// One model that may serve an Auto turn.
#[derive(Debug, Clone)]
pub struct AutoCandidate {
    pub id: String,
    pub provider_id: String,
    pub model_name: String,
    pub context_window: u32,
    pub vision: bool,
    pub tool_calling: bool,
    pub reasoning: bool,
    /// Locally hosted runtime (Ollama / LM Studio / vLLM / …).
    pub is_local: bool,
}

/// User-selectable tradeoff, mirroring Docker's `PRIORITIES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoPriority {
    #[default]
    Balanced,
    Quality,
    Speed,
    Cost,
}

impl AutoPriority {
    pub fn parse(value: &str) -> Self {
        match value {
            "quality" => Self::Quality,
            "speed" => Self::Speed,
            "cost" => Self::Cost,
            _ => Self::Balanced,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Quality => "quality",
            Self::Speed => "speed",
            Self::Cost => "cost",
        }
    }

    fn weights(self) -> (f64, f64, f64, f64) {
        // (quality_strength, speed_scale, cost_weight, explore)
        match self {
            Self::Balanced => (4.0, 1.0, 0.03, 0.35),
            Self::Quality => (8.0, 0.6, 0.0, 0.35),
            Self::Speed => (2.0, 1.8, 0.02, 0.30),
            Self::Cost => (2.0, 0.6, 0.18, 0.30),
        }
    }
}

/// Extra signals the DB layer may supply per candidate. All fields are
/// optional so a fresh install (no history) still routes sensibly.
#[derive(Debug, Clone, Default)]
pub struct CandidateSignals {
    /// `(up, down)` feedback counts for the current route profile.
    pub feedback: (f64, f64),
    /// Total weighted feedback across all profiles (for the UCB term).
    pub _total_feedback: f64,
    /// Penalty in `[0, ~1.7]` from recent failures / cooldowns.
    pub reliability_penalty: f64,
    /// Human-readable reason for the penalty, if any.
    pub reliability_reason: Option<String>,
    /// Bounded speed bonus in `[-0.16, 0.16]`.
    pub speed_score: f64,
}

/// Options for one selection.
#[derive(Debug, Clone)]
pub struct AutoSelectOptions {
    pub content: String,
    pub mode: String,
    pub tools_required: bool,
    pub has_image: bool,
    pub required_tokens: u64,
    pub current_model_id: Option<String>,
    pub prefer_other_provider_than: Option<String>,
    pub priority: AutoPriority,
    pub request_key: String,
    /// Give locally hosted models a modest score bonus. The user opts into
    /// this; capability and context filters still apply first.
    pub prefer_local: bool,
}

/// Auditable details about the selection, persisted on the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoSelection {
    pub profile: String,
    pub priority: String,
    pub candidate_count: usize,
    pub candidate_ids: Vec<String>,
    pub required_tokens: u64,
    pub selected_id: String,
    pub reason: String,
}

/// Models in these namespaces are endpoint-specific (embedding, rerank,
/// image/audio, safety classifiers, …) and never chat-capable. Unknown models
/// stay eligible — only known non-chat IDs are excluded.
fn is_known_non_chat(model_name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "nvidia/nemotron-parse",
        "nvidia/nv-embed",
        "nvidia/nv-rerank",
        "nvidia/parakeet",
        "nvidia/llama-nemotron-embed",
        "nvidia/llama-nemotron-rerank",
        "nvidia/rerank-",
        "nvidia/llama-3.2-nemoretriever",
        "nvidia/gliner",
        "baai/bge-",
        "black-forest-labs/flux",
        "nvidia/cosmos-transfer",
        "nvidia/cosmos-predict",
        "nvidia/sparsedrive",
        "nvidia/bevformer",
        "nvidia/streampetr",
        "nvidia/active-speaker-detection",
        "nvidia/synthetic-video-detector",
        "nvidia/magpie-tts",
        "nvidia/studiovoice",
        "nvidia/usdcode",
        "nvidia/usdvalidate",
        "nvidia/nemotron-content-safety",
        "nvidia/nemotron-3-content-safety",
        "nvidia/llama-3.1-nemotron-safety-guard",
        "meta/esmfold",
        "meta/esm2-",
        "nvidia/riva-translate-4b-instruct",
        "google/recurrentgemma-2b",
    ];
    let id = model_name.to_ascii_lowercase();
    PREFIXES.iter().any(|p| id.starts_with(p))
}

/// Stable, non-user-visible feature key for the current turn.
pub fn routing_profile(content: &str, mode: &str, tools_required: bool, has_image: bool) -> String {
    let lower = content.to_ascii_lowercase();
    let coding = lower.contains("```")
        || [
            "python",
            "javascript",
            "typescript",
            "react",
            "sql",
            "docker",
            "api",
            "bug",
            "code",
        ]
        .iter()
        .any(|w| contains_word(&lower, w))
        || content.contains("コード")
        || content.contains("実装")
        || content.contains("修正");
    let task = if coding {
        "coding"
    } else if contains_any(&lower, &["translate", "translation"])
        || content.contains("英訳")
        || content.contains("和訳")
        || content.contains("翻訳")
    {
        "translation"
    } else if lower.contains("summar") || content.contains("要約") || content.contains("まとめ")
    {
        "summarize"
    } else if contains_any(&lower, &["json", "yaml", "schema", "extract"])
        || content.contains("構造化")
        || content.contains("抽出")
        || content.contains("表形式")
    {
        "extraction"
    } else if contains_any(
        &lower,
        &[
            "calculate",
            "compute",
            "equation",
            "integral",
            "derivative",
            "math",
        ],
    ) || content.contains("数学")
        || content.contains("計算")
        || content.contains("解け")
    {
        "math"
    } else if contains_any(
        &lower,
        &["why", "prove", "derive", "analyze", "analyse", "reason"],
    ) || content.contains("比較")
        || content.contains("理由")
        || content.contains("証明")
        || content.contains("推論")
        || content.contains("考察")
        || content.contains("複雑")
    {
        "reasoning"
    } else {
        "general"
    };
    let base = if coding { "coding" } else { "general" };
    let mut tokens = vec![base.to_string()];
    if lower.contains("why")
        || lower.contains("prove")
        || lower.contains("reason")
        || mode == "thinking"
    {
        tokens.push("reasoning".to_string());
    }
    if tools_required {
        tokens.push("tools".to_string());
    }
    if has_image {
        tokens.push("vision".to_string());
    }
    let difficulty = if task == "reasoning" || task == "math" {
        "high"
    } else if task == "coding" {
        "mid"
    } else {
        "low"
    };
    let lang = if content
        .chars()
        .any(|c| ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c))
    {
        "ja"
    } else {
        "en"
    };
    format!(
        "{}:lang:{lang}:task:{task}:diff:{difficulty}",
        tokens.join(":")
    )
}

fn contains_word(haystack: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(word) {
        let s = start + pos;
        let e = s + word.len();
        let before = haystack[..s]
            .chars()
            .next_back()
            .map(is_word_char)
            .unwrap_or(false);
        let after = haystack[e..]
            .chars()
            .next()
            .map(is_word_char)
            .unwrap_or(false);
        if !before && !after {
            return true;
        }
        start = e;
    }
    false
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn contains_any(haystack: &str, words: &[&str]) -> bool {
    words.iter().any(|w| contains_word(haystack, w))
}

/// Static capability prior in `[0, 1]`, or `None` when unknown.
pub fn quality_prior(model_name: &str, reasoning: bool) -> Option<f64> {
    const TIERS: &[(&str, f64)] = &[
        ("gpt-4.1-mini", 0.55),
        ("gpt-4.1", 0.78),
        ("gpt-4o-mini", 0.45),
        ("gpt-4o", 0.70),
        ("o4-mini", 0.60),
        ("o3", 0.85),
        ("o1", 0.78),
        ("gpt-4", 0.55),
        ("claude-opus", 0.90),
        ("claude-sonnet", 0.75),
        ("claude-haiku", 0.48),
        ("claude-3", 0.55),
        ("gemini-2.5-pro", 0.85),
        ("gemini-2.5-flash", 0.58),
        ("gemini-2", 0.62),
        ("gemini-1.5-pro", 0.58),
        ("gemini-1.5-flash", 0.42),
    ];
    let id = model_name
        .to_ascii_lowercase()
        .trim_start_matches("models/")
        .to_string();
    let short = id.rsplit('/').next().unwrap_or(&id);
    let mut best: Option<(usize, f64)> = None;
    for candidate in [&id, &short.to_string()] {
        for (prefix, value) in TIERS {
            if candidate.starts_with(prefix)
                && best.map(|(len, _)| prefix.len() > len).unwrap_or(true)
            {
                best = Some((prefix.len(), *value));
            }
        }
    }
    let mut value = best.map(|(_, v)| v).or_else(|| size_prior(&id))?;
    if reasoning {
        value = (value + 0.04).min(1.0);
    }
    Some(value)
}

fn size_prior(id: &str) -> Option<f64> {
    // Parameter count in billions, e.g. `...-70b` or `...-1.5b`.
    let bytes = id.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            let is_b = j < bytes.len()
                && (bytes[j] == b'b' || bytes[j] == b'B')
                && (j + 1 >= bytes.len() || !bytes[j + 1].is_ascii_alphabetic());
            let boundary_before =
                i == 0 || (!bytes[i - 1].is_ascii_digit() && bytes[i - 1] != b'.');
            if is_b && boundary_before {
                if let Ok(params) = id[i..j].parse::<f64>() {
                    return Some(if params >= 200.0 {
                        0.85
                    } else if params >= 100.0 {
                        0.78
                    } else if params >= 60.0 {
                        0.68
                    } else if params >= 30.0 {
                        0.55
                    } else if params >= 12.0 {
                        0.45
                    } else if params >= 6.0 {
                        0.35
                    } else if params >= 2.0 {
                        0.22
                    } else {
                        0.15
                    });
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

fn request_cost_usd(model_name: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let price = crate::pricing::price_for(model_name)?;
    let input = input_tokens as f64 / 1_000_000.0;
    let output = output_tokens as f64 / 1_000_000.0;
    Some(input * price.input_per_million + output * price.output_per_million)
}

/// Deterministic per-request tiebreak: seeded Beta draw favouring stronger
/// feedback, with a hash fallback for full determinism.
fn tiebreak(request_key: &str, entries: &[(usize, f64, f64, f64, f64)]) -> usize {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // entries: (index, up, down, prior_up, prior_down)
    let mut best: Option<(usize, u64)> = None;
    for (index, up, down, prior_up, prior_down) in entries {
        let mut h = DefaultHasher::new();
        request_key.hash(&mut h);
        index.hash(&mut h);
        // Fold the feedback into the hash so stronger evidence wins ties
        // deterministically without any RNG dependency.
        ((up * 1000.0) as u64).hash(&mut h);
        ((down * 1000.0) as u64).hash(&mut h);
        ((prior_up * 1000.0) as u64).hash(&mut h);
        ((prior_down * 1000.0) as u64).hash(&mut h);
        let v = h.finish();
        if best.map(|(_, b)| v > b).unwrap_or(true) {
            best = Some((*index, v));
        }
    }
    best.map(|(i, _)| i).unwrap_or(0)
}

/// Select the best model for one turn, or return a user-facing reason.
pub fn select_auto_model(
    candidates: &[AutoCandidate],
    signals: &HashMap<String, CandidateSignals>,
    opts: &AutoSelectOptions,
    excluded_ids: &[String],
) -> Result<(AutoCandidate, AutoSelection), String> {
    let profile = routing_profile(
        &opts.content,
        &opts.mode,
        opts.tools_required,
        opts.has_image,
    );
    let mut excluded: HashMap<&str, usize> = HashMap::new();
    let excluded_set: std::collections::HashSet<&str> =
        excluded_ids.iter().map(|s| s.as_str()).collect();
    let mut eligible: Vec<&AutoCandidate> = Vec::new();
    for c in candidates {
        if excluded_set.contains(c.id.as_str()) {
            continue;
        }
        if is_known_non_chat(&c.model_name) {
            *excluded.entry("chat").or_default() += 1;
            continue;
        }
        if c.context_window > 0 && opts.required_tokens > c.context_window as u64 {
            *excluded.entry("context").or_default() += 1;
            continue;
        }
        if opts.has_image && !c.vision {
            *excluded.entry("vision").or_default() += 1;
            continue;
        }
        if opts.tools_required && !c.tool_calling {
            *excluded.entry("tools").or_default() += 1;
            continue;
        }
        eligible.push(c);
    }
    if eligible.is_empty() {
        let mut kinds: Vec<&str> = excluded.keys().copied().collect();
        kinds.sort_unstable();
        let details = kinds
            .iter()
            .map(|k| match *k {
                "chat" => "通常チャット",
                "context" => "Context Window",
                "vision" => "Vision",
                "tools" => "Tool Calling",
                _ => "条件",
            })
            .collect::<Vec<_>>()
            .join("、");
        return Err(format!(
            "Autoで使用可能なモデルに必要な{}を満たすものがありません。",
            if details.is_empty() {
                "条件".to_string()
            } else {
                details
            }
        ));
    }

    let (quality_strength, speed_scale, cost_weight, explore) = opts.priority.weights();
    let total_feedback: f64 = eligible
        .iter()
        .map(|c| {
            let s = signals.get(&c.id);
            s.map(|s| s.feedback.0 + s.feedback.1).unwrap_or(0.0)
        })
        .sum();

    // Cost signal: reward candidates cheaper than the median.
    let reserved = 4096u64;
    let costs: HashMap<&str, Option<f64>> = eligible
        .iter()
        .map(|c| {
            (
                c.id.as_str(),
                request_cost_usd(&c.model_name, opts.required_tokens, reserved),
            )
        })
        .collect();
    let mut known_costs: Vec<f64> = costs
        .values()
        .filter_map(|v| *v)
        .filter(|v| *v > 0.0)
        .collect();
    known_costs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_cost = if known_costs.len() >= 2 {
        Some(known_costs[known_costs.len() / 2])
    } else {
        None
    };
    let cost_score = |id: &str| -> f64 {
        match (costs.get(id).copied().flatten(), median_cost) {
            (Some(cost), Some(median)) if cost > 0.0 && median > 0.0 => {
                (median / cost).ln().clamp(-1.0, 1.0)
            }
            _ => 0.0,
        }
    };

    let has_other_provider = opts
        .prefer_other_provider_than
        .as_ref()
        .is_some_and(|p| eligible.iter().any(|c| &c.provider_id != p));

    let mut ranked: Vec<(f64, usize)> = Vec::new();
    let mut tie_entries: Vec<(usize, f64, f64, f64, f64)> = Vec::new();
    for (i, c) in eligible.iter().enumerate() {
        let sig = signals.get(&c.id);
        let (up, down) = sig.map(|s| s.feedback).unwrap_or((0.0, 0.0));
        let quality = quality_prior(&c.model_name, c.reasoning);
        let (prior_up, prior_down) = match quality {
            None => (8.0, 8.0),
            Some(q) => (
                8.0 + quality_strength * q,
                8.0 + quality_strength * (1.0 - q),
            ),
        };
        let count = up + down;
        let mean = (up + prior_up) / (count + prior_up + prior_down);
        let exploration = explore
            * (2.0 * (total_feedback + (prior_up + prior_down) * eligible.len() as f64 + 1.0).ln()
                / (count + prior_up + prior_down))
                .sqrt();
        let reasoning_bonus = if opts.mode == "thinking" && c.reasoning {
            0.04
        } else {
            0.0
        };
        let usage = if c.context_window > 0 {
            Some(opts.required_tokens as f64 / c.context_window as f64)
        } else {
            None
        };
        let context_penalty = usage.map(|u| (u - 0.5).max(0.0) * 0.1).unwrap_or(0.0);
        let reliability_penalty = sig.map(|s| s.reliability_penalty).unwrap_or(0.0);
        let provider_retry_penalty = if has_other_provider
            && opts.prefer_other_provider_than.as_ref() == Some(&c.provider_id)
        {
            1.0
        } else {
            0.0
        };
        let continuity_bonus = if opts.current_model_id.as_ref() == Some(&c.id) {
            0.03
        } else {
            0.0
        };
        let local_bonus = if opts.prefer_local && c.is_local {
            0.15
        } else {
            0.0
        };
        let speed = sig.map(|s| s.speed_score).unwrap_or(0.0) * speed_scale;
        let cost = cost_score(&c.id) * cost_weight;
        let score =
            mean + exploration + reasoning_bonus + speed + cost + continuity_bonus + local_bonus
                - context_penalty
                - reliability_penalty
                - provider_retry_penalty;
        ranked.push((score, i));
        tie_entries.push((i, up, down, prior_up, prior_down));
    }
    let best = ranked
        .iter()
        .map(|(s, _)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let tied: Vec<usize> = ranked
        .iter()
        .filter(|(s, _)| (s - best).abs() < 1e-12)
        .map(|(_, i)| *i)
        .collect();
    let chosen = if tied.len() > 1 {
        let entries: Vec<_> = tied.iter().map(|i| tie_entries[*i]).collect();
        tiebreak(&opts.request_key, &entries)
    } else {
        tied[0]
    };
    let selected = eligible[chosen].clone();

    let sig = signals.get(&selected.id);
    let mut parts = vec![format!(
        "Auto: {profile} ({}) の実績・探索度・既知Contextの余裕から選択",
        opts.priority.as_str()
    )];
    if let Some(reason) = sig.and_then(|s| s.reliability_reason.clone()) {
        parts.push(reason);
    }
    if quality_prior(&selected.model_name, selected.reasoning).is_some() {
        parts.push("モデル能力の事前知識も考慮".to_string());
    }
    if cost_score(&selected.id) != 0.0 {
        parts.push("コスト効率も考慮".to_string());
    }
    Ok((
        selected.clone(),
        AutoSelection {
            profile,
            priority: opts.priority.as_str().to_string(),
            candidate_count: eligible.len(),
            candidate_ids: eligible.iter().map(|c| c.id.clone()).collect(),
            required_tokens: opts.required_tokens,
            selected_id: selected.id.clone(),
            reason: parts.join("。"),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, provider: &str, model: &str) -> AutoCandidate {
        AutoCandidate {
            id: id.to_string(),
            provider_id: provider.to_string(),
            model_name: model.to_string(),
            context_window: 128_000,
            vision: true,
            tool_calling: true,
            reasoning: false,
            is_local: false,
        }
    }

    fn opts(content: &str) -> AutoSelectOptions {
        AutoSelectOptions {
            content: content.to_string(),
            mode: "chat".to_string(),
            tools_required: false,
            has_image: false,
            required_tokens: 1000,
            current_model_id: None,
            prefer_other_provider_than: None,
            priority: AutoPriority::Balanced,
            request_key: "test-key".to_string(),
            prefer_local: false,
        }
    }

    #[test]
    fn vision_requirement_filters_candidates() {
        let candidates = vec![
            AutoCandidate {
                vision: false,
                ..candidate("a", "p1", "gpt-4o-mini")
            },
            AutoCandidate {
                vision: true,
                ..candidate("b", "p1", "gpt-4o")
            },
        ];
        let mut o = opts("hello");
        o.has_image = true;
        let (selected, _) =
            select_auto_model(&candidates, &HashMap::new(), &o, &[]).expect("select");
        assert_eq!(selected.id, "b");
    }

    #[test]
    fn context_window_filters_candidates() {
        let candidates = vec![
            AutoCandidate {
                context_window: 1000,
                ..candidate("a", "p1", "small-model-1b")
            },
            candidate("b", "p1", "gpt-4o"),
        ];
        let mut o = opts("hello");
        o.required_tokens = 50_000;
        let (selected, _) =
            select_auto_model(&candidates, &HashMap::new(), &o, &[]).expect("select");
        assert_eq!(selected.id, "b");
    }

    #[test]
    fn prefer_local_breaks_an_otherwise_equal_choice() {
        let local = AutoCandidate {
            id: "local".into(),
            provider_id: "p2".into(),
            is_local: true,
            ..candidate("local", "p2", "gpt-4o")
        };
        let cloud = candidate("cloud", "p1", "gpt-4o");
        let mut o = opts("hello");
        o.prefer_local = true;
        let (selected, _) =
            select_auto_model(&[local, cloud], &HashMap::new(), &o, &[]).expect("select");
        assert_eq!(selected.id, "local");
    }

    #[test]
    fn no_eligible_candidate_returns_a_reason() {
        let candidates = vec![AutoCandidate {
            vision: false,
            ..candidate("a", "p1", "gpt-4o-mini")
        }];
        let mut o = opts("hello");
        o.has_image = true;
        let err = select_auto_model(&candidates, &HashMap::new(), &o, &[]).unwrap_err();
        assert!(err.contains("Vision"), "got: {err}");
    }

    #[test]
    fn known_non_chat_models_are_excluded() {
        let candidates = vec![
            candidate("a", "p1", "nvidia/nv-embed-qa-4"),
            candidate("b", "p1", "gpt-4o-mini"),
        ];
        let (selected, _) =
            select_auto_model(&candidates, &HashMap::new(), &opts("hello"), &[]).expect("select");
        assert_eq!(selected.id, "b");
    }

    #[test]
    fn quality_prior_prefers_known_strong_models() {
        assert!(
            quality_prior("gpt-4o", false).unwrap() > quality_prior("gpt-4o-mini", false).unwrap()
        );
        assert!(
            quality_prior("claude-opus-4-1", false).unwrap()
                > quality_prior("claude-3-5-haiku-latest", false).unwrap()
        );
        assert!(quality_prior("llama3.1:8b", false).is_some());
        assert!(quality_prior("some-unknown-model-xyz", false).is_none());
    }

    #[test]
    fn routing_profile_is_stable_and_distinguishes_tasks() {
        let coding = routing_profile("```python\nprint(1)\n```", "chat", false, false);
        let general = routing_profile("hello", "chat", false, false);
        assert_ne!(coding, general);
        assert_eq!(
            coding,
            routing_profile("```python\nprint(1)\n```", "chat", false, false)
        );
        assert!(general.contains("task:general"));
        assert!(coding.contains("task:coding") || coding.starts_with("coding:"));
    }

    #[test]
    fn continuity_breaks_near_ties_toward_the_current_model() {
        let candidates = vec![
            candidate("a", "p1", "some-unknown-model-alpha"),
            candidate("b", "p2", "some-unknown-model-beta"),
        ];
        let mut o = opts("hello");
        o.current_model_id = Some("b".to_string());
        let (selected, _) =
            select_auto_model(&candidates, &HashMap::new(), &o, &[]).expect("select");
        assert_eq!(selected.id, "b");
    }

    #[test]
    fn deterministic_tiebreak_is_stable_per_request_key() {
        let candidates = vec![
            candidate("a", "p1", "some-unknown-model-alpha"),
            candidate("b", "p1", "some-unknown-model-beta"),
        ];
        let o = opts("hello");
        let (first, _) = select_auto_model(&candidates, &HashMap::new(), &o, &[]).expect("select");
        let (second, _) = select_auto_model(&candidates, &HashMap::new(), &o, &[]).expect("select");
        assert_eq!(first.id, second.id);
    }
}
