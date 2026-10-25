//! MCP client host.
//!
//! Implements the Model Context Protocol client side over two transports:
//!
//! - **stdio** — spawns the server as a child process and speaks
//!   newline-delimited JSON-RPC 2.0 over stdin/stdout.
//! - **Streamable HTTP** — POSTs JSON-RPC to an endpoint and reads the SSE
//!   stream (or a single JSON response) back.
//!
//! A hand-rolled client is used instead of the `rmcp` SDK so the Desktop build
//! has no proc-macro surface and the protocol version we speak stays auditable.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{timeout, Duration};

use crate::error::McpError;

/// MCP protocol revision this client advertises.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Default overall deadline for one Streamable-HTTP JSON-RPC exchange, headers
/// and body together. A server that trickles bytes forever would otherwise hold
/// a `tools/call` open indefinitely, since the shared HTTP client only bounds
/// the gap between reads. Generous enough for genuinely slow tool work, and
/// overridable per connection via [`ConnectionConfig::http_timeout_secs`].
pub const DEFAULT_HTTP_CALL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
    StreamableHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub transport: Transport,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub headers: HashMap<String, String>,
    /// Per-connection deadline for one Streamable-HTTP exchange, in seconds.
    /// Defaults to [`DEFAULT_HTTP_CALL_TIMEOUT`] when absent, so a server with
    /// genuinely slow tool work can raise it without changing the global cap.
    #[serde(default)]
    pub http_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub content: Value,
    pub is_error: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionStatus {
    Configured,
    Connecting,
    Ready,
    Failed { reason: String },
    Disabled,
}

impl ConnectionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectionStatus::Configured => "configured",
            ConnectionStatus::Connecting => "connecting",
            ConnectionStatus::Ready => "ready",
            ConnectionStatus::Failed { .. } => "failed",
            ConnectionStatus::Disabled => "disabled",
        }
    }
}

/// Result of a JSON-RPC call: either the `result` payload or a typed error.
type RpcResult = Result<Value, McpError>;

/// A live MCP session.
pub struct Session {
    pub config: ConnectionConfig,
    pub status: RwLock<ConnectionStatus>,
    pub tools: RwLock<Vec<McpTool>>,
    stdin: Mutex<Option<ChildStdin>>,
    pending: Arc<Mutex<HashMap<i64, mpsc::Sender<RpcResult>>>>,
    next_id: Mutex<i64>,
    child: Mutex<Option<Child>>,
}

impl Session {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            status: RwLock::new(ConnectionStatus::Configured),
            tools: RwLock::new(Vec::new()),
            stdin: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Mutex::new(1),
            child: Mutex::new(None),
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        // The HTTP transport resolves the request inline, so it hands the
        // parsed response straight back instead of writing to a channel.
        if self.config.transport == Transport::StreamableHttp {
            let raw = self.post_http(&payload.to_string()).await?;
            return Self::unwrap_rpc(raw, method);
        }

        let (tx, mut rx) = mpsc::channel(1);
        self.pending.lock().await.insert(id, tx);
        if let Err(err) = self.write_stdio(&payload.to_string()).await {
            self.pending.lock().await.remove(&id);
            return Err(err);
        }
        match timeout(Duration::from_secs(60), rx.recv()).await {
            Ok(Some(Ok(v))) => Ok(v),
            Ok(Some(Err(e))) => Err(e),
            // The reader dropped the sender without replying. Remove the
            // pending entry so a later request with a reused id is not
            // shadowed by a dead channel.
            Ok(None) => {
                self.pending.lock().await.remove(&id);
                Err(McpError::ChannelClosed(method.to_string()))
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(McpError::Timeout(method.to_string()))
            }
        }
    }

    /// Turn a raw JSON-RPC envelope into either a result or an error.
    fn unwrap_rpc(raw: Value, method: &str) -> Result<Value, McpError> {
        if let Some(err) = raw.get("error") {
            return Err(McpError::Server(
                err.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown MCP error")
                    .to_string(),
            ));
        }
        raw.get("result")
            .cloned()
            .ok_or_else(|| McpError::MissingResult(method.to_string()))
    }

    async fn write_stdio(&self, body: &str) -> Result<(), McpError> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or(McpError::NotConnected)?;
        stdin.write_all(body.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await.map_err(McpError::Io)
    }

    /// POST a JSON-RPC frame and read the reply (JSON body or SSE frame),
    /// bounded by an overall deadline so a misbehaving server cannot hang a
    /// tool call forever.
    async fn post_http(&self, body: &str) -> Result<Value, McpError> {
        let deadline = self
            .config
            .http_timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_HTTP_CALL_TIMEOUT);
        match timeout(deadline, self.post_http_inner(body)).await {
            Ok(result) => result,
            Err(_) => Err(McpError::Timeout("streamable-http request".to_string())),
        }
    }

    async fn post_http_inner(&self, body: &str) -> Result<Value, McpError> {
        let url = self.config.url.clone().ok_or(McpError::MissingUrl)?;
        let client = mixd_providers::transport::client();
        let mut req = client
            .post(&url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(body.to_string());
        for (k, v) in &self.config.headers {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(McpError::HttpStatus(status.to_string(), text));
        }
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if content_type.contains("text/event-stream") {
            let mut buffer = String::new();
            let mut stream = resp.bytes_stream();
            use futures_util::StreamExt;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk?;
                buffer.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(idx) = buffer.find('\n') {
                    let line: String = buffer.drain(..=idx).collect();
                    if let Some(payload) = line.trim().strip_prefix("data:") {
                        if let Ok(value) = serde_json::from_str::<Value>(payload.trim()) {
                            return Ok(value);
                        }
                    }
                }
            }
            // The server hung up mid-conversation: worth a line of its own,
            // because it is indistinguishable from an idle server otherwise.
            tracing::warn!(
                connection = %self.config.id,
                "MCP SSE stream ended without a JSON-RPC response"
            );
            Err(McpError::SseEnded)
        } else {
            Ok(resp.json::<Value>().await?)
        }
    }

    /// Fire-and-forget notification (no `id`, no response expected).
    async fn notify(&self, method: &str) -> Result<(), McpError> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        match self.config.transport {
            Transport::Stdio => self.write_stdio(&payload.to_string()).await,
            Transport::StreamableHttp => self.post_http(&payload.to_string()).await.map(|_| ()),
        }
    }

    /// Perform the MCP handshake and fetch the tool list.
    pub async fn connect(&self) -> Result<(), McpError> {
        *self.status.write().await = ConnectionStatus::Connecting;

        if self.config.transport == Transport::Stdio {
            let command = self
                .config
                .command
                .clone()
                .ok_or(McpError::MissingCommand)?;
            validate_stdio_command(&command)?;
            let mut child = match Command::new(&command)
                .args(&self.config.args)
                .kill_on_drop(true)
                .spawn()
            {
                Ok(child) => child,
                Err(err) => {
                    // The single most common MCP failure (a `command` that is
                    // not installed) used to be invisible in the log.
                    tracing::warn!(
                        connection = %self.config.id,
                        command = %command,
                        %err,
                        "spawning the MCP server failed"
                    );
                    return Err(McpError::SpawnFailed(command, err.to_string()));
                }
            };
            let stdin = child.stdin.take().ok_or(McpError::NoStdin)?;
            let stdout = child.stdout.take().ok_or(McpError::NoStdout)?;
            *self.stdin.lock().await = Some(stdin);
            *self.child.lock().await = Some(child);

            // Pump stdout in the background.
            let pending = self.pending.clone();
            let reader_id = self.config.id.clone();
            tauri_free_spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                        continue;
                    };
                    // Server → client request or notification: nothing to answer.
                    if value.get("method").is_some() {
                        continue;
                    }
                    let Some(id) = value.get("id").and_then(|v| v.as_i64()) else {
                        continue;
                    };
                    let response = if let Some(err) = value.get("error") {
                        Err(McpError::Server(
                            err.get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("unknown MCP error")
                                .to_string(),
                        ))
                    } else {
                        Ok(value.get("result").cloned().unwrap_or(Value::Null))
                    };
                    if let Some(tx) = pending.lock().await.remove(&id) {
                        let _ = tx.send(response).await;
                    }
                }
                // Normal exit, a crash, or a read error all land here, and all
                // three leave the session unable to answer another request.
                tracing::warn!(connection = %reader_id, "MCP stdio reader stopped");
            });
        }

        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "roots": { "listChanged": false } },
                    "clientInfo": { "name": "mix-agent-desktop", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await;

        let init = match init {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(
                    connection = %self.config.id,
                    %err,
                    "MCP initialize handshake failed"
                );
                *self.status.write().await = ConnectionStatus::Failed {
                    reason: err.to_string(),
                };
                return Err(err);
            }
        };
        let _ = init;

        // `notifications/initialized` is a notification: no id, no response.
        let _ = self.notify("notifications/initialized").await;

        let listed = match self.request("tools/list", json!({})).await {
            Ok(listed) => listed,
            Err(err) => {
                tracing::warn!(
                    connection = %self.config.id,
                    %err,
                    "MCP tools/list failed after a successful handshake"
                );
                return Err(err);
            }
        };
        let tools = listed
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|t| McpTool {
                        name: t
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        description: t
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        input_schema: t
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({"type": "object"})),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let tool_count = tools.len();
        *self.tools.write().await = tools;
        *self.status.write().await = ConnectionStatus::Ready;
        tracing::info!(
            connection = %self.config.id,
            tools = tool_count,
            "MCP session connected"
        );
        Ok(())
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<ToolCallResult, McpError> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await?;
        Ok(ToolCallResult {
            is_error: result
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            content: result.get("content").cloned().unwrap_or(Value::Null),
        })
    }

    pub async fn shutdown(&self) {
        if self.config.transport == Transport::Stdio {
            *self.stdin.lock().await = None;
            if let Some(mut child) = self.child.lock().await.take() {
                // A leaked MCP server process is invisible without this: the
                // `Child` handle is dropped right after, so nothing else can
                // report that the kill failed.
                if let Err(err) = child.start_kill() {
                    tracing::warn!(
                        connection = %self.config.id,
                        %err,
                        "failed to kill the MCP server process"
                    );
                }
                // Reap the child, but never let a wedged process stall
                // shutdown.
                let _ = timeout(Duration::from_secs(5), child.wait()).await;
            }
        }
        *self.status.write().await = ConnectionStatus::Disabled;
        tracing::info!(connection = %self.config.id, "MCP session shut down");
    }

    pub async fn tool_specs(&self) -> Vec<mixd_providers::ToolSpec> {
        self.tools
            .read()
            .await
            .iter()
            .map(|t| mixd_providers::ToolSpec {
                name: format!("mcp__{}__{}", self.config.id, t.name),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            })
            .collect()
    }
}

/// Validate a stdio server `command` before it reaches `Command::new`.
///
/// The command comes from stored user configuration, but a poisoned or
/// mistyped row must not become shell interpretation or a path traversal:
/// reject empty commands, shell metacharacters (`; & | $ \``), and relative
/// paths. Only an absolute path (which must exist) or a bare binary name
/// resolved via `PATH` is accepted; anything containing `/` must be absolute.
pub fn validate_stdio_command(command: &str) -> Result<(), McpError> {
    if command.trim().is_empty() {
        return Err(McpError::SpawnFailed(
            command.to_string(),
            "stdio command is empty".to_string(),
        ));
    }
    const DANGEROUS: &[char] = &[';', '&', '|', '$', '`'];
    if command.chars().any(|c| DANGEROUS.contains(&c)) {
        return Err(McpError::SpawnFailed(
            command.to_string(),
            "stdio command contains shell metacharacters".to_string(),
        ));
    }
    if command.contains('/') {
        if !command.starts_with('/') {
            return Err(McpError::SpawnFailed(
                command.to_string(),
                "relative paths are not allowed; use an absolute path or a bare binary name"
                    .to_string(),
            ));
        }
        if !std::path::Path::new(command).exists() {
            return Err(McpError::SpawnFailed(
                command.to_string(),
                "absolute path does not exist".to_string(),
            ));
        }
    }
    Ok(())
}

/// Spawn a plain tokio task; the crate deliberately avoids the Tauri runtime
/// so it stays usable from tests and the CLI.
fn tauri_free_spawn<F: std::future::Future<Output = ()> + Send + 'static>(fut: F) {
    tokio::spawn(fut);
}

pub struct McpHost {
    pub sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl Default for McpHost {
    fn default() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }
}

impl McpHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn add(&self, config: ConnectionConfig) -> Arc<Session> {
        let session = Arc::new(Session::new(config.clone()));
        self.sessions
            .write()
            .await
            .insert(config.id.clone(), session.clone());
        session
    }

    pub async fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) -> Option<Arc<Session>> {
        let session = self.sessions.write().await.remove(id);
        if let Some(ref s) = session {
            s.shutdown().await;
        }
        session
    }

    pub async fn list(&self) -> Vec<(String, String, ConnectionStatus, usize)> {
        let guard = self.sessions.read().await;
        let mut out: Vec<(String, String, ConnectionStatus, usize)> = guard
            .values()
            .map(|s| {
                (
                    s.config.id.clone(),
                    s.config.name.clone(),
                    ConnectionStatus::Configured,
                    0,
                )
            })
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    /// Connect every enabled session and report the aggregated tool count.
    pub async fn connect_all(&self) -> anyhow::Result<usize> {
        let sessions: Vec<Arc<Session>> = self.sessions.read().await.values().cloned().collect();
        let mut total = 0;
        for session in sessions {
            match session.connect().await {
                Ok(()) => {
                    total += session.tools.read().await.len();
                }
                Err(reason) => {
                    tracing::warn!(
                        connection = %session.config.id,
                        %reason,
                        "MCP session could not be connected"
                    );
                    *session.status.write().await = ConnectionStatus::Failed {
                        reason: reason.to_string(),
                    };
                }
            }
        }
        Ok(total)
    }

    /// Route a Tool call to the owning MCP session.
    pub async fn dispatch(
        &self,
        qualified_name: &str,
        arguments: Value,
    ) -> Result<ToolCallResult, McpError> {
        let mut parts = qualified_name.splitn(3, "__");
        let _ = parts.next();
        let connection_id = parts.next().ok_or(McpError::MalformedToolName)?;
        let tool_name = parts.next().ok_or(McpError::MalformedToolName)?;
        let session = self
            .get(connection_id)
            .await
            .ok_or_else(|| McpError::ConnectionNotFound(connection_id.to_string()))?;
        session.call_tool(tool_name, arguments).await
    }

    /// Verify a server is reachable without registering it.
    pub async fn probe(config: &ConnectionConfig) -> anyhow::Result<usize> {
        let session = Arc::new(Session::new(config.clone()));
        session.connect().await.map_err(anyhow::Error::from)?;
        let count = session.tools.read().await.len();
        Ok(count)
    }
}

impl std::fmt::Debug for ConnectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Convenience for the command surface: parse a config out of stored JSON.
pub fn config_from_json(id: &str, value: &Value) -> anyhow::Result<ConnectionConfig> {
    let transport = match value
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or("stdio")
    {
        "streamable_http" => Transport::StreamableHttp,
        _ => Transport::Stdio,
    };
    // A remote endpoint has to be an absolute URL. `reqwest` only rejects a
    // relative or malformed one at request time, so the row used to look
    // "configured" and fail later with an opaque transport error; parse it now
    // instead. This is the one case where the parser can return `Err`.
    let url = match value.get("url").and_then(|v| v.as_str()) {
        Some(raw) => {
            if transport == Transport::StreamableHttp {
                url::Url::parse(raw).map_err(|err| McpError::InvalidUrl(raw.to_string(), err))?;
            }
            Some(raw.to_string())
        }
        None => None,
    };
    let args = value
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let headers = value
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Ok(ConnectionConfig {
        id: id.to_string(),
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_string(),
        transport,
        command: value
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        args,
        url,
        headers,
        http_timeout_secs: value.get("http_timeout_secs").and_then(|v| v.as_u64()),
    })
}

/// Build a config from the Docker edition's stored shape, with a context for
/// better error messages.
pub fn parse_connection(value: &Value) -> anyhow::Result<ConnectionConfig> {
    config_from_json("probe", value).context("parsing MCP connection")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stdio_config() {
        let v = json!({
            "name": "files",
            "transport": "stdio",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
        });
        let cfg = parse_connection(&v).expect("parse");
        assert_eq!(cfg.transport, Transport::Stdio);
        assert_eq!(cfg.args.len(), 3);
    }

    #[test]
    fn parses_http_config() {
        let v = json!({
            "name": "remote",
            "transport": "streamable_http",
            "url": "https://mcp.example.com/mcp",
            "headers": { "Authorization": "Bearer x" },
        });
        let cfg = parse_connection(&v).expect("parse");
        assert_eq!(cfg.transport, Transport::StreamableHttp);
        assert_eq!(
            cfg.headers.get("Authorization").map(String::as_str),
            Some("Bearer x")
        );
    }

    #[test]
    fn relative_stdio_path_is_rejected() {
        let err =
            validate_stdio_command("tools/server").expect_err("relative path must be rejected");
        assert!(err.to_string().contains("relative"), "unexpected: {err}");
    }

    #[test]
    fn metacharacters_in_stdio_command_are_rejected() {
        for bad in ["npx; rm -rf /", "node & evil", "x|y", "echo $HOME", "a`b`"] {
            assert!(
                validate_stdio_command(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
        // Bare binary names remain allowed.
        assert!(validate_stdio_command("npx").is_ok());
    }
}
