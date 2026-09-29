//! MCP server registry: loads stored connections and keeps the live
//! [`McpHost`] in sync.

use crate::host::{ConnectionConfig, McpHost};
use anyhow::Context;
use serde_json::Value;
use sqlx::Row;
use std::sync::Arc;

const SECRETS_NAME: &str = "headers";

/// Read every stored `mcp_connections` row into a config list.
///
/// `secrets` is optional because the Docker-edition parser and unit tests load
/// configs without a Keychain; when it is present, header values are read from
/// there and any row still holding them in plaintext is migrated out.
pub async fn load_configs(
    storage: &mixd_storage::Storage,
    secrets: Option<&mixd_storage::SecretStore>,
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
        if let Ok(mut config) = crate::host::config_from_json(&id, &value) {
            restore_headers(storage, secrets, &id, &value, &mut config).await;
            out.push(config);
        } else {
            // Silently dropping the row here used to make a stored connection
            // vanish from the host with no trace anywhere.
            tracing::warn!(
                connection = %id,
                "skipping stored MCP connection that failed to parse"
            );
        }
    }
    Ok(out)
}

/// Move header values between the row and the Keychain.
///
/// `Authorization` headers used to be stored verbatim in SQLite, which made
/// the Keychain encryption of OAuth tokens pointless: the equally sensitive
/// header sat one query away in plaintext.
async fn restore_headers(
    storage: &mixd_storage::Storage,
    secrets: Option<&mixd_storage::SecretStore>,
    id: &str,
    value: &Value,
    config: &mut ConnectionConfig,
) {
    let purpose = format!("mcp/{}", id);
    let plaintext: Vec<(String, String)> = value
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    if value.get("headersSecured").and_then(|v| v.as_bool()) != Some(true) && !plaintext.is_empty()
    {
        match secrets {
            Some(store) => {
                let encoded = match serde_json::to_vec(&plaintext) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        tracing::error!(connection = %id, %err, "encoding MCP headers failed");
                        return;
                    }
                };
                match store.put(&purpose, SECRETS_NAME, &encoded).await {
                    Ok(()) => {
                        if let Err(err) = sqlx::query(
                            "UPDATE mcp_connections \
                             SET data = json_set(data, '$.headers', NULL, '$.headersSecured', \
                                 json('true')) \
                             WHERE id = ?",
                        )
                        .bind(id)
                        .execute(storage.pool())
                        .await
                        {
                            tracing::error!(connection = %id, %err, "clearing plaintext MCP headers failed");
                            return;
                        }
                        tracing::info!(connection = %id, "MCP headers moved to the Keychain");
                    }
                    Err(err) => {
                        // Keep working from the plaintext copy this session;
                        // dropping it now would break the connection outright.
                        tracing::error!(connection = %id, %err, "storing MCP headers failed");
                        config.headers.extend(plaintext);
                        return;
                    }
                }
            }
            None => {
                tracing::warn!(
                    connection = %id,
                    "MCP headers are stored in plaintext; no Keychain available to migrate them"
                );
                config.headers.extend(plaintext);
                return;
            }
        }
    }

    let Some(store) = secrets else { return };
    if let Ok(encoded) = store.get_string(&purpose, SECRETS_NAME).await {
        match serde_json::from_str::<Vec<(String, String)>>(&encoded) {
            Ok(pairs) => config.headers.extend(pairs),
            Err(err) => {
                tracing::error!(connection = %id, %err, "stored MCP headers are unreadable")
            }
        }
    }
}

/// Populate a host from storage and connect every session.
pub async fn hydrate_and_connect(
    host: &Arc<McpHost>,
    storage: &mixd_storage::Storage,
    secrets: Option<&mixd_storage::SecretStore>,
) -> anyhow::Result<usize> {
    let configs = load_configs(storage, secrets).await.map_err(|err| {
        tracing::error!(%err, "hydrating the MCP host from storage failed");
        err
    })?;
    for config in configs {
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
