//! Tool Registry and executor.
//!
//! Each built-in Tool implements the [`Tool`] trait. The [`registry`] holds the
//! canonical tool list for each mode (Chat / Thinking / Agent) and the
//! [`execute`] module serialises side-effecting operations.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod agent;
pub mod approval;
pub mod edit;
pub mod egress;
pub mod execute;
pub mod files;
pub mod image_vision;
pub mod network;
pub mod permission;
pub mod registry;
pub mod search;
pub mod skill;
pub mod terminal;
pub mod todo;
pub mod user_plan;
pub mod web;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    pub run_id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    /// Absolute folder the run is scoped to. `None` → process CWD.
    #[serde(default)]
    pub root_path: Option<String>,
    /// Extra folders the agent may read from but not write to. Parity with
    /// Codex's `--add-dir` flag for monorepos.
    #[serde(default)]
    pub additional_paths: Vec<String>,
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub ok: bool,
    pub content: serde_json::Value,
    pub error: Option<String>,
}

/// Tool spec handed to a Provider when the model is offered the tool list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    fn default_permission(&self) -> permission::PermissionDecision {
        permission::PermissionDecision::Ask
    }

    /// Whether this particular invocation needs explicit user approval.
    ///
    /// The default is `true`, and the previous behaviour was equivalent to
    /// leaving it there for every tool: a fresh install asked permission before
    /// its first `glob`, which is both unusable and fatal for an unattended run
    /// — a scheduled run blocks on the first read and never finishes. Tools that
    /// only observe override this; tools that change something, reach the
    /// network, or run a command do not.
    ///
    /// This only decides the *fallback* when no rule is stored. A stored
    /// allow/deny rule always wins, so a user can still lock a read-only tool
    /// down.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        true
    }

    /// The decision to use for this specific call when nothing is stored.
    fn decision_for(&self, args: &serde_json::Value) -> permission::PermissionDecision {
        if !self.is_sensitive(args) {
            permission::PermissionDecision::Allow
        } else {
            self.default_permission()
        }
    }

    /// The string a `pattern`-scoped permission rule should be matched against.
    ///
    /// Most tools look at a path or URL in their arguments. Tools that have no
    /// obvious target (e.g. `todo`) return `None`, which means a pattern rule
    /// cannot match them and the rule falls through to the next bucket.
    fn pattern_target(&self, _args: &serde_json::Value) -> Option<String> {
        None
    }

    async fn invoke(&self, ctx: ToolContext, args: serde_json::Value)
        -> anyhow::Result<ToolResult>;
}

pub use egress::EgressPolicy;
pub use execute::PermissionContext;
pub use permission::{PermissionDecision, PermissionRule, RuleScope};
pub use registry::{McpDispatch, ToolRegistry};
