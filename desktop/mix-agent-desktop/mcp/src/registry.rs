//! MCP server registry: loads stored connections and keeps the live
//! [`McpHost`](crate::host::McpHost) in sync.

use crate::host::{ConnectionConfig, McpHost};
use anyhow::Context;
use serde_json::Value;
use sqlx::Row;
use std::sync::Arc;

/// Read every stored `mcp_connections` row into a config list.
pub async fn load_configs(
    storage: &mixd_storage::Storage,
) -> anyhow::Result<Vec<ConnectionConfig>> {
    let rows = sqlx::query("SELECT id, data FROM mcp_connections ORDER BY created_at")
        .fetch_all(storage.pool())
        .await
        .context("loading MCP connections")?;
    let mut out = Vec::new();
    for row in rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let raw: String = row.try_get("data").unwrap_or_else(|_| "{}".into());
        let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        if value.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        if let Ok(config) = crate::host::config_from_json(&id, &value) {
            out.push(config);
        }
    }
    Ok(out)
}

/// Populate a host from storage and connect every session.
pub async fn hydrate_and_connect(
    host: &Arc<McpHost>,
    storage: &mixd_storage::Storage,
) -> anyhow::Result<usize> {
    for config in load_configs(storage).await? {
        host.add(config).await;
    }
    host.connect_all().await
}

/// Aggregate the live Tool specs so the Run Engine can advertise them.
pub async fn collect_tool_specs(host: &Arc<McpHost>) -> Vec<mixd_providers::ToolSpec> {
    let sessions: Vec<_> = host.sessions.read().await.values().cloned().collect();
    let mut out = Vec::new();
    for session in sessions {
        out.extend(session.tool_specs().await);
    }
    out
}
