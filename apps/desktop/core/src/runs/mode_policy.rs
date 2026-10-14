//! Mode policy flags that toggle Agent-mode autonomy features.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModePolicy {
    pub checkpointing: bool,
    pub checkpoint_every_steps: u32,
    pub stagnation_detection: bool,
    pub strict_verification: bool,
    pub budget_extension: bool,
    pub background_processes: bool,
    pub persistent_browser: bool,
    pub context_tiered_summary: bool,
}

impl Default for AgentModePolicy {
    fn default() -> Self {
        Self {
            checkpointing: true,
            checkpoint_every_steps: 10,
            stagnation_detection: true,
            strict_verification: true,
            budget_extension: true,
            background_processes: true,
            persistent_browser: true,
            context_tiered_summary: true,
        }
    }
}
