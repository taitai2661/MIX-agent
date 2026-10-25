//! Run mode (Chat / Thinking / Plan / Agent) plus per-mode policy defaults.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Chat,
    Thinking,
    /// Reads and reasons but refuses every mutating Tool call, so a plan can be
    /// produced without touching the project. Enforced in the Run Engine's Tool
    /// loop, not by withholding the Tools — the model still needs `files.read`
    /// and `grep` to write a plan worth reading.
    Plan,
    Agent,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Chat => "chat",
            Mode::Thinking => "thinking",
            Mode::Plan => "plan",
            Mode::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "chat" => Some(Mode::Chat),
            "thinking" => Some(Mode::Thinking),
            "plan" => Some(Mode::Plan),
            "agent" => Some(Mode::Agent),
            _ => None,
        }
    }
    /// `true` when the mode must not change anything on disk.
    ///
    /// A method on the mode rather than a field on
    /// [`crate::runs::mode_policy::AgentModePolicy`]: a policy flag could be
    /// edited to `false` while the mode stayed `plan`, and the refusal would
    /// silently stop happening.
    pub fn is_read_only(&self) -> bool {
        matches!(self, Mode::Plan)
    }

    pub fn default_step_limit(&self) -> u32 {
        match self {
            Mode::Chat => 12,
            Mode::Thinking => 24,
            Mode::Plan => 60,
            Mode::Agent => 300,
        }
    }

    pub fn default_tool_call_limit(&self) -> u32 {
        match self {
            Mode::Chat => 12,
            Mode::Thinking => 24,
            Mode::Plan => 150,
            Mode::Agent => 750,
        }
    }

    pub fn default_wall_clock_minutes(&self) -> u32 {
        match self {
            Mode::Chat => 20,
            Mode::Thinking => 45,
            Mode::Plan => 45,
            Mode::Agent => 90,
        }
    }

    /// Default token budget per mode, or `0` for unlimited.
    ///
    /// Steps bound how many *turns* a Run gets; tokens bound how much context
    /// it may burn doing so. The two diverge badly on a large repository, where
    /// a handful of reads can cost more than a hundred small turns.
    pub fn default_token_limit(&self) -> u32 {
        match self {
            Mode::Chat => 0,
            Mode::Thinking => 500_000,
            Mode::Plan => 800_000,
            Mode::Agent => 4_000_000,
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
            .ok_or_else(|| format!("unknown mode: {value} (expected chat|thinking|plan|agent)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_round_trips_and_is_read_only() {
        let mode = Mode::parse("plan").expect("plan must parse");
        assert_eq!(mode.as_str(), "plan");
        assert!(mode.is_read_only());
        assert_eq!(Mode::parse(mode.as_str()), Some(mode));
    }

    #[test]
    fn the_writable_modes_are_not_read_only() {
        for mode in [Mode::Chat, Mode::Thinking, Mode::Agent] {
            assert!(!mode.is_read_only(), "{mode:?} must be writable");
        }
    }
}
