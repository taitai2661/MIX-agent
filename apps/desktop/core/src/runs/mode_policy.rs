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
    /// How long an `ask_user` card waits for a steering answer, in seconds.
    /// Kept in the policy instead of a literal in the dispatch path so a
    /// deployment can tune it without recompiling the engine.
    #[serde(default = "default_ask_user_timeout_secs")]
    pub ask_user_timeout_secs: u64,
    /// How many read-only Tool calls may run in parallel in one turn.
    /// Bounded rather than unbounded: twenty greps must not become twenty
    /// directory walks competing for the same disk. Default 4 is safe on HDDs,
    /// network mounts and container volumes as well as NVMe.
    #[serde(default = "default_max_concurrent_reads")]
    pub max_concurrent_reads: usize,
}

fn default_ask_user_timeout_secs() -> u64 {
    300
}

fn default_max_concurrent_reads() -> usize {
    4
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
            ask_user_timeout_secs: default_ask_user_timeout_secs(),
            max_concurrent_reads: default_max_concurrent_reads(),
        }
    }
}
