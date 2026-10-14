//! MIX-agent Desktop — Run Engine core.
//!
//! `mixd-core` owns the Run lifecycle, the per-Run state machine, the
//! SSE-equivalent event stream, and the long-running Agent autonomy features
//! (checkpointing, stagnation detection, strict verification, hierarchical
//! context summarisation, and budget extension).

pub mod auto;
pub mod context;
pub mod instructions;
pub mod knowledge;
pub mod memory;
pub mod memory_jobs;
pub mod memory_lifecycle;
pub mod memory_links;
pub mod mode;
pub mod pricing;
pub mod prompt;
pub mod providers_bridge;
pub mod recall_fixture;
pub mod reliability;
pub mod retrieval;
pub mod routing;
pub mod runs;
pub mod schedule;
pub mod skills;

pub use mode::Mode;
pub use runs::engine::RunEngine;
pub use runs::state::{RunId, RunStatus, RunView};
