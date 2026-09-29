use crate::AppState;
use mixd_mcp::host::{config_from_json, Transport};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct McpConnectionView {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub enabled: bool,
    pub status: String,
    pub tool_count: usize,
}

#[tauri::command]
pub async fn list_connections(
    state: State<'_, AppState>,
) -> Result<Vec<McpConnectionView>, String> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT id, data FROM mcp_connections ORDER BY created_at DESC")
        .fetch_all(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    let mut out = Vec::new();
    for row in rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let data: serde_json::Value = row
            .try_get::<String, _>("data")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let live = state.mcp.get(&id).await;
        let (status, tool_count) = match live {
            Some(session) => {
                let status = session.status.read().await.as_str().to_string();
                let count = session.tools.read().await.len();
                (status, count)
            }
            None => (
                data.get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("configured")
                    .to_string(),
                0,
            ),
        };
        out.push(McpConnectionView {
            name: data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string(),
            transport: data
                .get("transport")
                .and_then(|v| v.as_str())
                .unwrap_or("stdio")
                .to_string(),
            enabled: data
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            status,
            tool_count,
            id,
        });
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct McpConnectionCreate {
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub url: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
}

#[tauri::command]
pub async fn create_connection(
    state: State<'_, AppState>,
    input: McpConnectionCreate,
) -> Result<McpConnectionView, String> {
    let id = uuid::Uuid::new_v4().to_string();
    // `headers` carries `Authorization` and friends: those values go to the
    // OS Keychain, and the row only records that they exist.
    let mut data = serde_json::json!({
        "name": input.name,
        "transport": input.transport,
        "command": input.command,
        "args": input.args,
        "url": input.url,
        "enabled": true,
        "status": "configured",
    });
    if let Some(headers) = input.headers.filter(|h| !h.is_empty()) {
        let encoded = serde_json::to_vec(&headers).map_err(crate::error::cmd_err)?;
        state
            .secrets
            .put(&format!("mcp/{}", id), "headers", &encoded)
            .await
            .map_err(crate::error::cmd_err)?;
        data["headersSecured"] = serde_json::json!(true);
    }
    sqlx::query("INSERT INTO mcp_connections (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(McpConnectionView {
        id: id.clone(),
        name: input.name,
        transport: input.transport,
        enabled: true,
        status: "configured".to_string(),
        tool_count: 0,
    })
}

#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.mcp.remove(&id).await;
    sqlx::query("DELETE FROM mcp_connections WHERE id = ?")
        .bind(&id)
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    if let Err(err) = state
        .secrets
        .delete(&format!("mcp/{}", id), "headers")
        .await
    {
        tracing::error!(connection = %id, %err, "removing stored MCP headers failed");
    }
    refresh_registry(&state).await;
    Ok(())
}

/// Rebuild the engine's Tool Registry from the live MCP host.
///
/// `with_mcp` alone only makes `mcp__*` calls dispatchable; `set_mcp_specs` is
/// what puts them in the list the Provider is shown. Without that second step,
/// connecting a server changed the tool count in the UI and nothing else — the
/// model was never told the tools existed.
///
/// The registry is rebuilt through [`crate::build_registry`] rather than from
/// scratch: assembling it here once dropped the `skill` and `agent` tools and
/// reset the egress allowlist, so every MCP operation quietly uninstalled half
/// the agent's capabilities until the next launch.
async fn refresh_registry(state: &AppState) {
    let egress = crate::load_egress_policy(state.storage.as_ref()).await;
    let mut tools = crate::build_registry(
        &state.storage,
        &state.browser,
        &state.mcp,
        &state.agent_runner,
        egress,
    );
    tools.set_mcp_specs(
        mixd_mcp::collect_tool_specs(&state.mcp)
            .await
            .into_iter()
            .map(|spec| mixd_tools::ToolSpec {
                name: spec.name,
                description: spec.description,
                parameters: spec.parameters,
            })
            .collect(),
    );
    state.engine.replace_tools(tools).await;
}

/// (Re)connect every configured MCP server and report the total tool count.
#[tauri::command]
pub async fn connect(state: State<'_, AppState>) -> Result<usize, String> {
    let count = mixd_mcp::hydrate_and_connect(
        &state.mcp,
        state.storage.as_ref(),
        Some(state.secrets.as_ref()),
    )
    .await
    .map_err(crate::error::cmd_err)?;
    refresh_registry(&state).await;
    Ok(count)
}

#[derive(Serialize)]
pub struct OAuthStartOutput {
    pub authorization_url: String,
    pub state: String,
}

/// Begin the OAuth 2.0 + PKCE flow for a remote MCP server.
#[tauri::command]
pub async fn start_oauth(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<OAuthStartOutput, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT data FROM mcp_connections WHERE id = ?")
        .bind(&connection_id)
        .fetch_optional(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    let data: serde_json::Value = row
        .and_then(|(raw,)| serde_json::from_str(&raw).ok())
        .ok_or("connection not found")?;
    let base = data
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("remote MCP server needs a url")?;
    let origin = url::Url::parse(base)
        .map_err(crate::error::cmd_err)?
        .origin()
        .ascii_serialization();
    let endpoints = mixd_mcp::oauth::OAuthEndpoints {
        authorization_endpoint: format!("{}/authorize", origin),
        token_endpoint: format!("{}/token", origin),
        client_id: data
            .get("clientId")
            .and_then(|v| v.as_str())
            .unwrap_or("mix-agent-desktop")
            .to_string(),
        scopes: vec!["mcp".to_string()],
    };
    let pkce = mixd_mcp::oauth::PkcePair::new();
    let request = mixd_mcp::oauth::AuthRequest {
        endpoints: endpoints.clone(),
        pkce: pkce.clone(),
        redirect_uri: "mix-agent-desktop://oauth".to_string(),
        resource: Some(base.to_string()),
    };
    // Persist the pending flow so the deep-link callback can finish it.
    sqlx::query("INSERT INTO mcp_auth_states (id, owner_id, data, state_hash) VALUES (?, ?, ?, ?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(
            serde_json::json!({
                "connectionId": connection_id,
                "verifier": pkce.verifier,
                "state": pkce.state,
                "redirectUri": "mix-agent-desktop://oauth",
            })
            .to_string(),
        )
        .bind(&pkce.state)
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    // Store the endpoints so the callback can find the token endpoint again.
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE mcp_connections SET data = json_set(data, '$.oauthEndpoints', ?) WHERE id = ?",
        )
        .bind(serde_json::to_string(&endpoints).unwrap_or_default())
        .bind(&connection_id)
        .execute(state.storage.pool())
        .await,
        "store MCP OAuth endpoints",
    );
    Ok(OAuthStartOutput {
        authorization_url: request.authorization_url(),
        state: pkce.state,
    })
}

/// Complete the OAuth flow from the `mix-agent-desktop://oauth` deep link.
#[tauri::command]
pub async fn handle_oauth_callback(state: State<'_, AppState>, url: String) -> Result<(), String> {
    complete_oauth_flow(&state, &url).await
}

/// Finish a pending OAuth flow for a `mix-agent-desktop://oauth?...` redirect.
///
/// Shared by the Tauri command and the deep-link listener in `app/src/lib.rs`,
/// which receives the URL as a bare string with no `State` in scope.
pub async fn complete_oauth_flow(state: &AppState, url: &str) -> Result<(), String> {
    let callback = mixd_mcp::oauth::parse_callback(url).map_err(crate::error::cmd_err)?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT id, data FROM mcp_auth_states WHERE state_hash = ?")
            .bind(&callback.state)
            .fetch_optional(state.storage.pool())
            .await
            .map_err(crate::error::cmd_err)?;
    let (auth_id, data_raw) = row.ok_or("no pending OAuth flow for this state")?;
    let data: serde_json::Value = serde_json::from_str(&data_raw).unwrap_or_default();
    let connection_id = data
        .get("connectionId")
        .and_then(|v| v.as_str())
        .ok_or("auth state is missing connectionId")?
        .to_string();
    let verifier = data
        .get("verifier")
        .and_then(|v| v.as_str())
        .ok_or("auth state is missing verifier")?
        .to_string();
    let redirect_uri = data
        .get("redirectUri")
        .and_then(|v| v.as_str())
        .unwrap_or("mix-agent-desktop://oauth")
        .to_string();

    let conn_row: Option<(String,)> =
        sqlx::query_as("SELECT data FROM mcp_connections WHERE id = ?")
            .bind(&connection_id)
            .fetch_optional(state.storage.pool())
            .await
            .map_err(crate::error::cmd_err)?;
    let conn_data: serde_json::Value = conn_row
        .and_then(|(raw,)| serde_json::from_str(&raw).ok())
        .ok_or("connection not found")?;
    let endpoints: mixd_mcp::oauth::OAuthEndpoints = conn_data
        .get("oauthEndpoints")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or("connection has no OAuth endpoints")?;

    let pkce = mixd_mcp::oauth::PkcePair {
        verifier,
        challenge: String::new(),
        state: callback.state.clone(),
    };
    let http = mixd_providers::transport::client();
    let tokens =
        mixd_mcp::oauth::exchange_code(http, &endpoints, &pkce, &redirect_uri, &callback.code)
            .await
            .map_err(crate::error::cmd_err)?;
    let access = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("token response has no access_token")?;
    state
        .secrets
        .put(
            &format!("mcp/{}", connection_id),
            "access_token",
            access.as_bytes(),
        )
        .await
        .map_err(crate::error::cmd_err)?;
    if let Some(refresh) = tokens.get("refresh_token").and_then(|v| v.as_str()) {
        if let Err(err) = state
            .secrets
            .put(
                &format!("mcp/{}", connection_id),
                "refresh_token",
                refresh.as_bytes(),
            )
            .await
        {
            // Without the refresh token the access token cannot be renewed and
            // the connection silently degrades on expiry.
            tracing::error!(%connection_id, %err, "storing the MCP refresh token failed");
        }
    }
    mixd_storage::log_write(
        sqlx::query("DELETE FROM mcp_auth_states WHERE id = ?")
            .bind(auth_id)
            .execute(state.storage.pool())
            .await,
        "clear MCP OAuth state",
    );
    // A newly authorised server may expose tools that were not listed before.
    if let Err(err) = mixd_mcp::hydrate_and_connect(
        &state.mcp,
        state.storage.as_ref(),
        Some(state.secrets.as_ref()),
    )
    .await
    {
        tracing::warn!(?err, "MCP reconnect after OAuth failed");
    }
    refresh_registry(state).await;
    Ok(())
}

/// Re-exported so the app shell can name the enum.
pub fn transport_label(transport: Transport) -> &'static str {
    match transport {
        Transport::Stdio => "stdio",
        Transport::StreamableHttp => "streamable_http",
    }
}

/// Helper used by the shell when it needs to rebuild a config from storage.
pub fn build_config(
    id: &str,
    data: &serde_json::Value,
) -> anyhow::Result<mixd_mcp::ConnectionConfig> {
    config_from_json(id, data)
}
