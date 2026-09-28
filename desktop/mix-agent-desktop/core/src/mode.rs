//! Run mode (Chat / Thinking / Agent) plus per-mode policy defaults.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Chat,
    Thinking,
    Agent,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Chat => "chat",
            Mode::Thinking => "thinking",
            Mode::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "chat" => Some(Mode::Chat),
            "thinking" => Some(Mode::Thinking),
            "agent" => Some(Mode::Agent),
            _ => None,
        }
    }

    pub fn default_step_limit(&self) -> u32 {
        match self {
            Mode::Chat => 12,
            Mode::Thinking => 24,
            Mode::Agent => 300,
        }
    }

    pub fn default_tool_call_limit(&self) -> u32 {
        match self {
            Mode::Chat => 12,
            Mode::Thinking => 24,
            Mode::Agent => 750,
        }
    }

    pub fn default_wall_clock_minutes(&self) -> u32 {
        match self {
            Mode::Chat => 20,
            Mode::Thinking => 45,
            Mode::Agent => 90,
        }
    }
}
