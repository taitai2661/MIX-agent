//! Provider routing & capability probe.
//!
//! Cost-aware step routing (B9): a Run serves different steps with different
//! model tiers instead of pinning one model for the whole Run.
//!
//! ```text
//! Local  — on-device / LAN runtime (Ollama, LM Studio, …). $0, never leaves
//!          the machine. Best for compression, exploration sweeps, follow-ups.
//! Cheap  — small cloud models (mini / haiku / flash class). Cents per run.
//! Strong — frontier models. Reserved for analysis and the final answer.
//! ```
//!
//! Design notes:
//!
//! - Step classification is a cheap heuristic, not a planner: exploration
//!   sweeps, verification follow-ups and history compression are
//!   cheap-model-friendly (they rephrase or emit read-only calls), while
//!   analysis (Thinking / Plan) and the concluding turns stay on Strong.
//! - Budget pressure *downgrades* tiers before the hard budget gate fires:
//!   token spend vs `token_limit` and monthly spend vs the configured cap both
//!   push routing toward cheaper tiers. Pressure ≥ 0.7 downgrades one tier,
//!   ≥ 0.9 forces the cheapest tier available.
//! - Pure functions only. The engine resolves *availability* (which tier
//!   models are actually configured — Auto runs pre-resolve them at dispatch)
//!   and performs the switch; this module only decides. That keeps routing
//!   unit-testable without a database or a provider registry.

use serde::{Deserialize, Serialize};

use crate::mode::Mode;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    #[default]
    Unknown,
    Supported,
    Unsupported,
}

/// Cost tier one step of a Run may be served by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepTier {
    Local,
    Cheap,
    Strong,
}

impl StepTier {
    pub fn as_str(self) -> &'static str {
        match self {
            StepTier::Local => "local",
            StepTier::Cheap => "cheap",
            StepTier::Strong => "strong",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "cheap" => Some(Self::Cheap),
            "strong" => Some(Self::Strong),
            _ => None,
        }
    }

    /// Cheaper rank first: Local < Cheap < Strong.
    fn rank(self) -> u8 {
        match self {
            StepTier::Local => 0,
            StepTier::Cheap => 1,
            StepTier::Strong => 2,
        }
    }

    /// One step cheaper, saturating at [`StepTier::Local`].
    pub fn downgraded(self) -> Self {
        match self {
            StepTier::Strong => StepTier::Cheap,
            StepTier::Cheap => StepTier::Local,
            StepTier::Local => StepTier::Local,
        }
    }
}

/// One pre-resolved model serving a tier. Resolved by the app layer at
/// dispatch (Auto runs: primary + fallbacks classified by
/// [`classify_candidate`]); the engine only switches between these.
#[derive(Debug, Clone)]
pub struct TierModel {
    pub tier: StepTier,
    pub model_id: String,
    pub provider_id: String,
    pub context_window: u32,
    pub keep_alive: Option<String>,
}

/// The base tier for a turn, before budget pressure is applied.
///
/// `steps_taken` is the number of provider turns already served (0 for the
/// first turn). `verification_pending` covers both halves of the strict
/// verification gate: an unconfirmed destructive effect, or changed files the
/// project's own check has not blessed yet. Either way the next turn only has
/// to read something back and report it — no frontier model required.
pub fn base_tier(mode: Mode, steps_taken: u32, verification_pending: bool) -> StepTier {
    if verification_pending {
        return StepTier::Cheap;
    }
    match mode {
        // Analysis is the product: keep it on the strongest model available.
        Mode::Thinking | Mode::Plan => StepTier::Strong,
        // Early agent steps are usually an exploration sweep (reads, greps,
        // listings). Emitting read-only calls needs tool discipline, not
        // reasoning depth.
        Mode::Agent if steps_taken < 2 => StepTier::Cheap,
        // Chat is short-lived; quality first, pressure still downgrades.
        _ => StepTier::Strong,
    }
}

/// Combined budget pressure in `[0, 1]`: the worse of the token-ceiling spend
/// ratio and the monthly-cap spend ratio. Either one approaching 1 means the
/// corresponding hard gate is about to fire.
pub fn budget_pressure(token_ratio: f32, monthly_ratio: f32) -> f32 {
    token_ratio.max(monthly_ratio).clamp(0.0, 1.0)
}

/// Downgrade `base` under `pressure` (see [`budget_pressure`]).
///
/// - `pressure >= 0.9` forces [`StepTier::Local`]: the run is one turn away
///   from a hard stop, so only a $0 model keeps it alive.
/// - `pressure >= 0.7` downgrades one tier: Strong → Cheap → Local.
/// - Below that the base tier stands.
pub fn apply_pressure(base: StepTier, pressure: f32) -> StepTier {
    if pressure >= 0.9 {
        StepTier::Local
    } else if pressure >= 0.7 {
        base.downgraded()
    } else {
        base
    }
}

/// One decision, auditable on the `step_routing` event.
#[derive(Debug, Clone)]
pub struct TierDecision {
    pub base: StepTier,
    pub tier: StepTier,
    pub pressure: f32,
}

impl TierDecision {
    pub fn reason(&self) -> String {
        if self.tier != self.base {
            format!(
                "予算圧力 {:.0}% のため {} → {} にダウングレード",
                self.pressure * 100.0,
                self.base.as_str(),
                self.tier.as_str()
            )
        } else {
            format!(
                "{} tier（予算圧力 {:.0}%）",
                self.tier.as_str(),
                self.pressure * 100.0
            )
        }
    }
}

/// Decide the tier for one turn.
pub fn decide_tier(
    mode: Mode,
    steps_taken: u32,
    verification_pending: bool,
    token_ratio: f32,
    monthly_ratio: f32,
) -> TierDecision {
    let base = base_tier(mode, steps_taken, verification_pending);
    let pressure = budget_pressure(token_ratio, monthly_ratio);
    let tier = apply_pressure(base, pressure);
    TierDecision {
        base,
        tier,
        pressure,
    }
}

/// Name fragments that mark the small/cheap cloud class.
///
/// The price table is a small, deliberately stale static list, so a price-only
/// test misclassifies models whose entry is a little above the threshold —
/// `o4-mini` is listed at $1.10/M and was being served on the Strong tier.
/// The model family name is the primary signal; price remains the secondary
/// one for cheap models that do not carry one of these markers.
const CHEAP_NAME_FRAGMENTS: &[&str] = &["mini", "haiku", "flash", "deepseek", "gpt-3.5"];

/// Classify a configured model into a tier.
///
/// A local runtime is always [`StepTier::Local`] (estimated cost zero). For a
/// cloud model the family name is checked first ([`CHEAP_NAME_FRAGMENTS`]),
/// then its listed input price below $1/M. An unlisted cloud model with no
/// cheap marker is Strong: never assume cheap.
pub fn classify_candidate(is_local: bool, model_name: &str) -> StepTier {
    if is_local {
        return StepTier::Local;
    }
    let lowered = model_name.to_ascii_lowercase();
    // "gemini" contains the substring "mini", so a bare fragment test would
    // classify `gemini-2.5-pro` as cheap. Neutralise that one family prefix;
    // its flash variant still matches on "flash".
    let haystack = lowered.replace("gemini", "gem");
    if CHEAP_NAME_FRAGMENTS
        .iter()
        .any(|fragment| haystack.contains(fragment))
    {
        return StepTier::Cheap;
    }
    match crate::pricing::price_for(model_name) {
        Some(price) if price.input_per_million < 1.0 => StepTier::Cheap,
        _ => StepTier::Strong,
    }
}

/// Exact tier lookup. Used for preferences (e.g. the summariser) where
/// falling back to a *more expensive* tier would defeat the purpose.
pub fn tier_model_exact(models: &[TierModel], tier: StepTier) -> Option<TierModel> {
    models.iter().find(|m| m.tier == tier).cloned()
}

/// Pick the model to serve `requested`: exact match first, otherwise the
/// nearest tier by rank distance, ties toward the cheaper tier.
///
/// Returns `None` when no tier model is configured at all (fixed-model runs),
/// in which case the caller keeps the current model.
pub fn select_tier_model(requested: StepTier, models: &[TierModel]) -> Option<TierModel> {
    if models.is_empty() {
        return None;
    }
    if let Some(exact) = tier_model_exact(models, requested) {
        return Some(exact);
    }
    models
        .iter()
        .min_by_key(|m| {
            let dist = m.tier.rank().abs_diff(requested.rank());
            (dist, m.tier.rank())
        })
        .cloned()
}

/// Tiers acceptable for history-compression turns, in preference order.
///
/// Compression rephrases rather than reasons, so the cheapest configured
/// model does it. Deliberately exact-match only: falling back to Strong here
/// would spend frontier money on the turn that exists to *save* money.
pub const SUMMARY_TIER_PREFERENCE: [StepTier; 2] = [StepTier::Local, StepTier::Cheap];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_follow_ups_are_cheap() {
        assert_eq!(
            base_tier(Mode::Agent, 50, true),
            StepTier::Cheap,
            "a read-back turn needs no frontier model"
        );
        assert_eq!(base_tier(Mode::Thinking, 3, true), StepTier::Cheap);
    }

    #[test]
    fn analysis_modes_stay_strong() {
        assert_eq!(base_tier(Mode::Thinking, 0, false), StepTier::Strong);
        assert_eq!(base_tier(Mode::Plan, 5, false), StepTier::Strong);
    }

    #[test]
    fn early_agent_steps_explore_cheap() {
        assert_eq!(base_tier(Mode::Agent, 0, false), StepTier::Cheap);
        assert_eq!(base_tier(Mode::Agent, 1, false), StepTier::Cheap);
        assert_eq!(
            base_tier(Mode::Agent, 2, false),
            StepTier::Strong,
            "later steps decide and conclude"
        );
    }

    #[test]
    fn pressure_downgrades_before_the_hard_gate() {
        assert_eq!(apply_pressure(StepTier::Strong, 0.0), StepTier::Strong);
        assert_eq!(apply_pressure(StepTier::Strong, 0.69), StepTier::Strong);
        assert_eq!(apply_pressure(StepTier::Strong, 0.7), StepTier::Cheap);
        assert_eq!(apply_pressure(StepTier::Cheap, 0.8), StepTier::Local);
        assert_eq!(
            apply_pressure(StepTier::Strong, 0.95),
            StepTier::Local,
            "one turn from a hard stop: only $0 keeps the run alive"
        );
        assert_eq!(apply_pressure(StepTier::Local, 1.0), StepTier::Local);
    }

    #[test]
    fn pressure_takes_the_worse_of_both_spends() {
        assert!((budget_pressure(0.9, 0.1) - 0.9).abs() < f32::EPSILON);
        assert!((budget_pressure(0.1, 0.9) - 0.9).abs() < f32::EPSILON);
        assert_eq!(budget_pressure(2.0, -1.0), 1.0);
    }

    #[test]
    fn local_is_always_local_and_unknown_is_strong() {
        assert_eq!(classify_candidate(true, "gpt-4o"), StepTier::Local);
        assert_eq!(
            classify_candidate(true, "anything-unknown",),
            StepTier::Local
        );
        assert_eq!(classify_candidate(false, "gpt-4o-mini"), StepTier::Cheap);
        assert_eq!(
            classify_candidate(false, "claude-3-5-haiku-latest"),
            StepTier::Cheap
        );
        // Priced at $1.10/M — above the old price-only threshold, but still a
        // "mini" model and therefore a cheap step candidate.
        assert_eq!(classify_candidate(false, "o4-mini"), StepTier::Cheap);
        assert_eq!(
            classify_candidate(false, "gemini-2.5-flash"),
            StepTier::Cheap
        );
        assert_eq!(classify_candidate(false, "gpt-4o"), StepTier::Strong);
        assert_eq!(
            classify_candidate(false, "gemini-2.5-pro"),
            StepTier::Strong
        );
        assert_eq!(
            classify_candidate(false, "some-unlisted-model-xyz"),
            StepTier::Strong,
            "never assume cheap"
        );
    }

    fn tier_model(tier: StepTier, id: &str) -> TierModel {
        TierModel {
            tier,
            model_id: id.to_string(),
            provider_id: "p".to_string(),
            context_window: 128_000,
            keep_alive: None,
        }
    }

    #[test]
    fn exact_match_wins_then_nearest_cheaper() {
        let models = vec![
            tier_model(StepTier::Cheap, "cheap"),
            tier_model(StepTier::Strong, "strong"),
        ];
        assert_eq!(
            select_tier_model(StepTier::Cheap, &models)
                .unwrap()
                .model_id,
            "cheap"
        );
        // Local requested, none configured: nearest is Cheap (dist 1), not Strong.
        assert_eq!(
            select_tier_model(StepTier::Local, &models)
                .unwrap()
                .model_id,
            "cheap"
        );
        // Strong requested, only Cheap configured: serve it rather than stall.
        assert_eq!(
            select_tier_model(StepTier::Strong, &[tier_model(StepTier::Cheap, "c")])
                .unwrap()
                .model_id,
            "c"
        );
        assert!(select_tier_model(StepTier::Strong, &[]).is_none());
    }

    #[test]
    fn tier_round_trips() {
        for tier in [StepTier::Local, StepTier::Cheap, StepTier::Strong] {
            assert_eq!(StepTier::parse(tier.as_str()), Some(tier));
        }
        assert_eq!(StepTier::parse("frontier"), None);
    }
}
