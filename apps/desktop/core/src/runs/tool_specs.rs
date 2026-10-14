//! Shared Tool-spec mapper for the Run Engine.
//!
//! Split out of `engine.rs` so the full-list and mode-filtered paths cannot
//! drift apart: both used to spell the same `mixd_tools::ToolSpec` →
//! `mixd_providers::ToolSpec` field copy inline, and a field added on one side
//! (e.g. a future `sensitive` flag) would silently be dropped by the other.

/// Map advertised tool specs into the provider wire shape.
pub fn map_tool_specs(specs: &[mixd_tools::ToolSpec]) -> Vec<mixd_providers::ToolSpec> {
    specs
        .iter()
        .map(|s| mixd_providers::ToolSpec {
            name: s.name.clone(),
            description: s.description.clone(),
            parameters: s.parameters.clone(),
        })
        .collect()
}
