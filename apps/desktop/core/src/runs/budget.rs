//! Per-Run execution budget with the budget-extension handshake.

use serde::{Deserialize, Serialize};

use crate::mode::Mode;

/// A Run's step / tool-call / wall-clock budget.
#[derive(Debug, Clone, Serialize)]
pub struct Budget {
    pub step_limit: u32,
    pub tool_call_limit: u32,
    pub wall_clock_minutes: u32,
    /// Token ceiling for the Run, or `0` for unlimited.
    ///
    /// Steps bound how many turns a Run gets; tokens bound how much context it
    /// may burn doing so. The two diverge badly on a large repository, where a
    /// handful of reads can cost more than a hundred small turns, so a step
    /// limit alone is not a spending limit.
    pub token_limit: u32,
    /// Input + output tokens reported by the Provider so far.
    pub tokens_used: u32,
    pub grants_used: u32,
    pub max_grants: u32,
    /// Separated child-run aggregation: totals reported by delegated sub-runs
    /// (see `record_child`). These never gate `check` — a child's spend does
    /// not consume the parent's step / tool-call / token allowance, it is only
    /// summed here so the transcript can account for the whole workflow.
    #[serde(default)]
    pub child_steps_total: u32,
    #[serde(default)]
    pub child_tool_calls_total: u32,
    #[serde(default)]
    pub child_tokens_total: u32,
    #[serde(skip)]
    pub started_at: std::time::Instant,
    #[serde(skip)]
    pub extension_timeout_secs: u64,
}

impl<'de> Deserialize<'de> for Budget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            step_limit: u32,
            tool_call_limit: u32,
            wall_clock_minutes: u32,
            // Defaulted rather than required: rows written before the token
            // budget existed must keep deserializing.
            #[serde(default)]
            token_limit: u32,
            #[serde(default)]
            tokens_used: u32,
            grants_used: u32,
            max_grants: u32,
            // Rows written before child-run aggregation existed omit these;
            // defaulting keeps them loadable and starts the sums at zero.
            #[serde(default)]
            child_steps_total: u32,
            #[serde(default)]
            child_tool_calls_total: u32,
            #[serde(default)]
            child_tokens_total: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            step_limit: raw.step_limit,
            tool_call_limit: raw.tool_call_limit,
            wall_clock_minutes: raw.wall_clock_minutes,
            token_limit: raw.token_limit,
            tokens_used: raw.tokens_used,
            grants_used: raw.grants_used,
            max_grants: raw.max_grants,
            child_steps_total: raw.child_steps_total,
            child_tool_calls_total: raw.child_tool_calls_total,
            child_tokens_total: raw.child_tokens_total,
            started_at: std::time::Instant::now(),
            extension_timeout_secs: 300,
        })
    }
}

/// Read a stored budget number without ever wrapping it.
///
/// `as u32` silently truncates: `2^32` became `0`, which for `tokenLimit` means
/// "unlimited" — a corrupt or hand-edited row could remove the spending gate
/// entirely. An out-of-range value must therefore *not* become `u32::MAX`
/// either: for a token ceiling that is effectively unlimited too (`check` only
/// fires at `tokens_used >= token_limit`, which nobody reaches at ~4 billion),
/// and `token_pressure` would report ~0 so step routing would never downgrade.
/// The safe answer is the caller-supplied `fallback` — the mode's ceiling for a
/// limit, `0` for a usage counter — which keeps the gate intact.
fn stored_u32(value: &serde_json::Value, key: &str, fallback: u32) -> u32 {
    match value.get(key).and_then(|v| v.as_u64()) {
        Some(v) => u32::try_from(v).unwrap_or_else(|_| {
            tracing::warn!(%v, field = key, %fallback,
                "stored budget value exceeds u32; falling back to the safe value");
            fallback
        }),
        None => fallback,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetVerdict {
    Ok,
    Exhausted(&'static str),
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            step_limit: 12,
            tool_call_limit: 12,
            wall_clock_minutes: 20,
            token_limit: 0,
            tokens_used: 0,
            grants_used: 0,
            max_grants: 2,
            child_steps_total: 0,
            child_tool_calls_total: 0,
            child_tokens_total: 0,
            started_at: std::time::Instant::now(),
            extension_timeout_secs: 300,
        }
    }
}

impl Budget {
    pub fn for_mode(mode: Mode) -> Self {
        Self {
            step_limit: mode.default_step_limit(),
            tool_call_limit: mode.default_tool_call_limit(),
            wall_clock_minutes: mode.default_wall_clock_minutes(),
            token_limit: mode.default_token_limit(),
            ..Default::default()
        }
    }

    /// Rebuild a budget from its stored JSON.
    ///
    /// The limits are *optional overrides* on top of `base`, which is why the
    /// caller passes the mode's budget rather than relying on the defaults here.
    /// Getting this wrong is not cosmetic: the UI sends `null` for any limit the
    /// user did not edit, and reading `null` as "use the default" meant that
    /// clicking "grant" on an agent run replaced its 300-step budget with the
    /// 12-step default — the user asked for more room and got less.
    pub fn from_json_with_base(value: &serde_json::Value, base: Self) -> Self {
        Self {
            step_limit: stored_u32(value, "stepLimit", base.step_limit),
            tool_call_limit: stored_u32(value, "toolCallLimit", base.tool_call_limit),
            wall_clock_minutes: stored_u32(value, "wallClockMinutes", base.wall_clock_minutes),
            // `null` (the UI's "not edited") keeps the mode's ceiling; a stored
            // `0` is a deliberate "unlimited" and is preserved.
            token_limit: stored_u32(value, "tokenLimit", base.token_limit),
            tokens_used: stored_u32(value, "tokensUsed", 0),
            grants_used: stored_u32(value, "grantsUsed", 0),
            max_grants: stored_u32(value, "maxGrants", base.max_grants),
            // Child aggregation is report-only: a stored parent keeps its own
            // sums, a fresh base starts them at zero.
            child_steps_total: stored_u32(value, "childStepsTotal", 0),
            child_tool_calls_total: stored_u32(value, "childToolCallsTotal", 0),
            child_tokens_total: stored_u32(value, "childTokensTotal", 0),
            // The wall clock restarts when a run is resumed or reloaded;
            // carrying the old elapsed time would immediately re-exhaust it.
            started_at: std::time::Instant::now(),
            extension_timeout_secs: base.extension_timeout_secs,
        }
    }

    pub fn from_json(value: &serde_json::Value) -> Self {
        Self::from_json_with_base(value, Self::default())
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "stepLimit": self.step_limit,
            "toolCallLimit": self.tool_call_limit,
            "wallClockMinutes": self.wall_clock_minutes,
            "tokenLimit": self.token_limit,
            "tokensUsed": self.tokens_used,
            "grantsUsed": self.grants_used,
            "maxGrants": self.max_grants,
            "childStepsTotal": self.child_steps_total,
            "childToolCallsTotal": self.child_tool_calls_total,
            "childTokensTotal": self.child_tokens_total,
        })
    }

    /// Budget for a delegated child run: fully separate from the parent's.
    ///
    /// A sub-agent inherits the mode shape (small, bounded) but never the
    /// parent's remaining allowance — sharing limits would let one fan-out of
    /// eight exhaust the run it belongs to. The parent aggregates what the
    /// children spent via [`Budget::record_child`]; it never deducts it.
    pub fn for_child() -> Self {
        Self::for_mode(Mode::Thinking)
    }

    /// Fold one finished child run into the separated counters.
    ///
    /// Deliberately deaf to the parent's own limits: `check` ignores these
    /// fields, so a large fan-out is visible in the transcript without
    /// tripping the parent's `step_limit` / `token_limit`.
    pub fn record_child(&mut self, steps: u32, tool_calls: u32, tokens: u32) {
        self.child_steps_total = self.child_steps_total.saturating_add(steps);
        self.child_tool_calls_total = self.child_tool_calls_total.saturating_add(tool_calls);
        self.child_tokens_total = self.child_tokens_total.saturating_add(tokens);
    }

    /// Record what a turn cost, so the token ceiling reflects the whole Run.
    pub fn record_usage(&mut self, input_tokens: u64, output_tokens: u64) {
        let spent = input_tokens.saturating_add(output_tokens);
        self.tokens_used = self
            .tokens_used
            .saturating_add(spent.min(u32::MAX as u64) as u32);
    }

    /// Fraction of the token ceiling spent, in `[0, 1]`.
    ///
    /// Feeds cost-aware step routing ([`crate::routing::budget_pressure`]):
    /// routing downgrades model tiers as this approaches 1, *before* `check`
    /// fires the hard stop. An unlimited ceiling (`0`) reports `0.0` — there
    /// is no gate approaching, so there is nothing to steer away from.
    pub fn token_pressure(&self) -> f32 {
        if self.token_limit == 0 {
            return 0.0;
        }
        // Both operands can be near u32::MAX; f32 has only 24 mantissa bits and
        // would lose precision on the division. Compute in f64 and clamp last.
        let ratio = self.tokens_used as f64 / self.token_limit as f64;
        (ratio as f32).clamp(0.0, 1.0)
    }

    pub async fn check(&mut self, steps: u32, tool_calls: u32) -> BudgetVerdict {
        if steps >= self.step_limit {
            return BudgetVerdict::Exhausted("step_limit");
        }
        if tool_calls >= self.tool_call_limit {
            return BudgetVerdict::Exhausted("tool_call_limit");
        }
        // `0` is unlimited, which is also what a build with no pricing data
        // configured reports.
        if self.token_limit > 0 && self.tokens_used >= self.token_limit {
            return BudgetVerdict::Exhausted("token_limit");
        }
        if self.started_at.elapsed()
            >= std::time::Duration::from_secs(self.wall_clock_minutes as u64 * 60)
        {
            return BudgetVerdict::Exhausted("wall_clock");
        }
        BudgetVerdict::Ok
    }

    /// Relax the budget after a granted extension.
    ///
    /// Returns `false` and changes nothing once the grant allowance is spent.
    /// `max_grants` used to be decorative: `can_extend` had no call sites, so a
    /// user repeatedly clicking "grant" could double a run's budget forever.
    pub fn extend(&mut self) -> bool {
        if !self.can_extend() {
            return false;
        }
        self.grants_used = self.grants_used.saturating_add(1);
        self.step_limit = self.step_limit.saturating_mul(2);
        self.tool_call_limit = self.tool_call_limit.saturating_mul(2);
        // An unlimited budget stays unlimited: doubling `0` would turn
        // "no ceiling" into a 0-token ceiling.
        if self.token_limit > 0 {
            self.token_limit = self.token_limit.saturating_mul(2);
        }
        self.wall_clock_minutes = self.wall_clock_minutes.saturating_add(30);
        self.started_at = std::time::Instant::now();
        true
    }

    pub fn can_extend(&self) -> bool {
        self.grants_used < self.max_grants
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn step_limit_blocks() {
        let mut b = Budget::default();
        assert_eq!(b.check(0, 0).await, BudgetVerdict::Ok);
        assert_eq!(b.check(12, 0).await, BudgetVerdict::Exhausted("step_limit"));
    }

    #[tokio::test]
    async fn a_token_ceiling_blocks_a_run_that_is_under_its_step_limit() {
        let mut b = Budget::for_mode(Mode::Agent);
        b.token_limit = 1_000;
        b.record_usage(600, 400);
        assert_eq!(b.tokens_used, 1_000);
        assert_eq!(b.check(1, 1).await, BudgetVerdict::Exhausted("token_limit"));
    }

    #[tokio::test]
    async fn an_unlimited_token_budget_never_blocks() {
        let mut b = Budget::default();
        b.record_usage(u64::MAX, u64::MAX);
        assert_eq!(b.check(0, 0).await, BudgetVerdict::Ok);
    }

    #[tokio::test]
    async fn extending_an_unlimited_budget_leaves_it_unlimited() {
        let mut b = Budget::default();
        assert_eq!(b.token_limit, 0);
        assert!(b.extend());
        assert_eq!(b.token_limit, 0, "doubling unlimited must stay unlimited");
    }

    #[tokio::test]
    async fn extending_a_limited_budget_doubles_the_token_ceiling() {
        let mut b = Budget::for_mode(Mode::Agent);
        let before = b.token_limit;
        assert!(before > 0);
        assert!(b.extend());
        assert_eq!(b.token_limit, before * 2);
    }

    #[test]
    fn a_stored_document_without_a_token_budget_still_loads() {
        let stored = serde_json::json!({
            "stepLimit": 40,
            "toolCallLimit": 80,
            "wallClockMinutes": 30,
            "grantsUsed": 0,
            "maxGrants": 2,
        });
        let restored = Budget::from_json_with_base(&stored, Budget::for_mode(Mode::Agent));
        assert_eq!(restored.step_limit, 40);
        assert_eq!(
            restored.token_limit,
            Mode::Agent.default_token_limit(),
            "an absent token limit keeps the mode's ceiling"
        );
    }

    #[tokio::test]
    async fn extend_doubles_limits() {
        let mut b = Budget::default();
        let before = b.step_limit;
        assert!(b.extend());
        assert_eq!(b.step_limit, before * 2);
        assert!(b.can_extend());
    }

    #[tokio::test]
    async fn the_grant_allowance_is_enforced() {
        let mut b = Budget::default();
        assert!(b.extend());
        assert!(b.extend());
        assert!(
            !b.can_extend(),
            "max_grants must actually stop the doubling"
        );
        let capped = b.step_limit;
        assert!(!b.extend(), "a third grant must be refused");
        assert_eq!(b.step_limit, capped, "a refused grant changes nothing");
    }

    #[test]
    fn null_overrides_keep_the_mode_budget() {
        // The UI sends `null` for limits the user did not edit. Reading that as
        // "fall back to the global default" turned a 300-step agent run into a
        // 12-step one the moment anyone clicked "grant".
        let agent = Budget::for_mode(Mode::Agent);
        assert_eq!(agent.step_limit, 300);
        let stored = serde_json::json!({
            "stepLimit": null,
            "toolCallLimit": null,
            "wallClockMinutes": null,
        });
        let restored = Budget::from_json_with_base(&stored, agent);
        assert_eq!(restored.step_limit, 300);
        assert_eq!(restored.tool_call_limit, 750);
        assert_eq!(restored.wall_clock_minutes, 90);
    }

    #[test]
    fn out_of_range_stored_values_fall_back_to_base_not_truncated() {
        let base = Budget::for_mode(Mode::Agent);
        let stored = serde_json::json!({
            "stepLimit": 4_294_967_296u64, // 2^32 -> 0 when cast to u32
            "tokenLimit": 18_446_744_073_709_551_615u64,
            "tokensUsed": 4_294_967_297u64,
        });
        let restored = Budget::from_json_with_base(&stored, base.clone());
        // `u32::MAX` would be just as much of an "unlimited" gate as the `0`
        // the old truncation produced. The mode's own ceiling is the safe value.
        assert_eq!(restored.step_limit, base.step_limit);
        assert_eq!(restored.token_limit, base.token_limit);
        assert!(restored.token_limit > 0, "the spending gate must survive");
        assert_eq!(restored.tokens_used, 0);
    }

    #[test]
    fn a_fallen_back_token_ceiling_still_drives_routing_pressure() {
        let base = Budget::for_mode(Mode::Agent);
        let stored = serde_json::json!({ "tokenLimit": 18_446_744_073_709_551_615u64 });
        let mut restored = Budget::from_json_with_base(&stored, base.clone());
        assert_eq!(restored.token_limit, base.token_limit);
        restored.record_usage(base.token_limit as u64 / 2, 0);
        assert!(
            (restored.token_pressure() - 0.5).abs() < 1e-6,
            "routing must still see the restored ceiling approaching"
        );
    }

    #[test]
    fn explicit_overrides_win() {
        let agent = Budget::for_mode(Mode::Agent);
        let stored = serde_json::json!({ "stepLimit": 42, "wallClockMinutes": 5 });
        let restored = Budget::from_json_with_base(&stored, agent);
        assert_eq!(restored.step_limit, 42);
        assert_eq!(restored.wall_clock_minutes, 5);
        assert_eq!(
            restored.tool_call_limit, 750,
            "an omitted field keeps the mode default"
        );
    }

    #[test]
    fn round_trips_through_json() {
        let mut b = Budget::for_mode(Mode::Thinking);
        b.extend();
        let restored = Budget::from_json_with_base(&b.to_json(), b.clone());
        assert_eq!(restored.step_limit, b.step_limit);
        assert_eq!(restored.grants_used, b.grants_used);
        assert_eq!(restored.max_grants, b.max_grants);
    }

    #[tokio::test]
    async fn child_spend_is_aggregated_without_consuming_the_parent() {
        let mut parent = Budget::default();
        parent.record_child(6, 9, 1_500);
        parent.record_child(4, 3, 500);
        assert_eq!(parent.child_steps_total, 10);
        assert_eq!(parent.child_tool_calls_total, 12);
        assert_eq!(parent.child_tokens_total, 2_000);
        // The parent's own allowance is untouched: eight parallel children
        // must not trip the run they belong to.
        assert_eq!(parent.tokens_used, 0);
        assert_eq!(parent.check(0, 0).await, BudgetVerdict::Ok);
        // ...while the parent's own spend still gates exactly as before.
        parent.record_usage(10, 5);
        assert_eq!(parent.tokens_used, 15);
        let restored = Budget::from_json_with_base(&parent.to_json(), Budget::default());
        assert_eq!(restored.child_steps_total, 10);
        assert_eq!(restored.child_tokens_total, 2_000);
    }

    #[test]
    fn a_stored_document_without_child_counters_still_loads() {
        let stored = serde_json::json!({
            "stepLimit": 12,
            "toolCallLimit": 12,
            "wallClockMinutes": 20,
            "grantsUsed": 0,
            "maxGrants": 2,
        });
        let restored = Budget::from_json_with_base(&stored, Budget::default());
        assert_eq!(restored.child_steps_total, 0);
        assert_eq!(restored.child_tokens_total, 0);
    }

    #[test]
    fn token_pressure_tracks_the_ceiling() {
        let mut b = Budget::for_mode(Mode::Agent);
        assert!(b.token_limit > 0);
        assert_eq!(b.token_pressure(), 0.0);
        b.record_usage(b.token_limit as u64 / 2, 0);
        assert!((b.token_pressure() - 0.5).abs() < 1e-6);
        b.record_usage(u64::MAX, u64::MAX);
        assert_eq!(b.token_pressure(), 1.0, "overspend clamps, never exceeds 1");
    }

    #[test]
    fn token_pressure_is_precise_for_large_ceilings() {
        let b = Budget {
            token_limit: u32::MAX,
            tokens_used: u32::MAX / 4,
            ..Default::default()
        };
        let pressure = b.token_pressure();
        assert!(
            (pressure - 0.25).abs() < 1e-5,
            "f32 mantissa loss must not distort large ratios: {pressure}"
        );
    }

    #[test]
    fn an_unlimited_ceiling_has_no_pressure() {
        let mut b = Budget::default();
        assert_eq!(b.token_limit, 0);
        b.record_usage(u64::MAX, u64::MAX);
        assert_eq!(
            b.token_pressure(),
            0.0,
            "no ceiling means no approaching gate to steer away from"
        );
    }
}
