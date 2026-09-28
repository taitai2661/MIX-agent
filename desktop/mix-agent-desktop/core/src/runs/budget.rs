//! Per-Run execution budget with the budget-extension handshake.

use serde::{Deserialize, Serialize};

use crate::mode::Mode;

/// A Run's step / tool-call / wall-clock budget.
#[derive(Debug, Clone, Serialize)]
pub struct Budget {
    pub step_limit: u32,
    pub tool_call_limit: u32,
    pub wall_clock_minutes: u32,
    pub grants_used: u32,
    pub max_grants: u32,
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
            grants_used: u32,
            max_grants: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            step_limit: raw.step_limit,
            tool_call_limit: raw.tool_call_limit,
            wall_clock_minutes: raw.wall_clock_minutes,
            grants_used: raw.grants_used,
            max_grants: raw.max_grants,
            started_at: std::time::Instant::now(),
            extension_timeout_secs: 300,
        })
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
            grants_used: 0,
            max_grants: 2,
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
        let pick = |key: &str, fallback: u32| {
            value
                .get(key)
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(fallback)
        };
        Self {
            step_limit: pick("stepLimit", base.step_limit),
            tool_call_limit: pick("toolCallLimit", base.tool_call_limit),
            wall_clock_minutes: pick("wallClockMinutes", base.wall_clock_minutes),
            grants_used: value
                .get("grantsUsed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            max_grants: pick("maxGrants", base.max_grants),
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
            "grantsUsed": self.grants_used,
            "maxGrants": self.max_grants,
        })
    }

    pub async fn check(&mut self, steps: u32, tool_calls: u32) -> BudgetVerdict {
        if steps >= self.step_limit {
            return BudgetVerdict::Exhausted("step_limit");
        }
        if tool_calls >= self.tool_call_limit {
            return BudgetVerdict::Exhausted("tool_call_limit");
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
}
