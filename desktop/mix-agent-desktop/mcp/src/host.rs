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

/// MCP protocol revision this client advertises.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

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
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    pub headers: HashMap<String, String>,
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

/// Result of a JSON-RPC call: either the `result` payload or an error string.
type RpcResult = Result<Value, String>;

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

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
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
            Ok(None) => Err(format!("MCP server closed the channel for {}", method)),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("MCP request timed out: {}", method))
            }
        }
    }

    /// Turn a raw JSON-RPC envelope into either a result or an error.
    fn unwrap_rpc(raw: Value, method: &str) -> Result<Value, String> {
        if let Some(err) = raw.get("error") {
            return Err(err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown MCP error")
                .to_string());
        }
        raw.get("result")
            .cloned()
            .ok_or_else(|| format!("MCP response for {} has no result", method))
    }

    async fn write_stdio(&self, body: &str) -> Result<(), String> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or("stdio session is not running")?;
        stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdin.write_all(b"\n").await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())
    }

    /// POST a JSON-RPC frame and read the reply (JSON body or SSE frame).
    async fn post_http(&self, body: &str) -> Result<Value, String> {
        let url = self
            .config
            .url
            .clone()
            .ok_or("streamable_http needs a url")?;
        let client = reqwest::Client::new();
        let mut req = client
            .post(&url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(body.to_string());
        for (k, v) in &self.config.headers {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status, text));
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
                let bytes = chunk.map_err(|e| e.to_string())?;
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
            Err("SSE stream ended without a JSON-RPC response".into())
        } else {
            resp.json::<Value>().await.map_err(|e| e.to_string())
        }
    }

    /// Fire-and-forget notification (no `id`, no response expected).
    async fn notify(&self, method: &str) -> Result<(), String> {
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
    pub async fn connect(&self) -> Result<(), String> {
        *self.status.write().await = ConnectionStatus::Connecting;

        if self.config.transport == Transport::Stdio {
            let command = self.config.command.clone().ok_or("stdio needs a command")?;
            let mut child = Command::new(&command)
                .args(&self.config.args)
                .envs(&self.config.env)
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("failed to spawn {}: {}", command, e))?;
            let stdin = child.stdin.take().ok_or("child has no stdin")?;
            let stdout = child.stdout.take().ok_or("child has no stdout")?;
            *self.stdin.lock().await = Some(stdin);
            *self.child.lock().await = Some(child);

            // Pump stdout in the background.
            let pending = self.pending.clone();
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
                        Err(err
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown MCP error")
                            .to_string())
                    } else {
                        Ok(value.get("result").cloned().unwrap_or(Value::Null))
                    };
                    if let Some(tx) = pending.lock().await.remove(&id) {
                        let _ = tx.send(response).await;
                    }
                }
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
            Err(e) => {
                *self.status.write().await = ConnectionStatus::Failed { reason: e.clone() };
                return Err(e);
            }
        };
        let _ = init;

        // `notifications/initialized` is a notification: no id, no response.
        let _ = self.notify("notifications/initialized").await;

        let listed = self.request("tools/list", json!({})).await?;
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
        *self.tools.write().await = tools;
        *self.status.write().await = ConnectionStatus::Ready;
        Ok(())
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolCallResult, String> {
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
                let _ = child.start_kill();
            }
        }
        *self.status.write().await = ConnectionStatus::Disabled;
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
                    *session.status.write().await = ConnectionStatus::Failed { reason };
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
    ) -> Result<ToolCallResult, String> {
        let mut parts = qualified_name.splitn(3, "__");
        let _ = parts.next();
        let connection_id = parts.next().ok_or("malformed MCP tool name")?;
        let tool_name = parts.next().ok_or("malformed MCP tool name")?;
        let session = self
            .get(connection_id)
            .await
            .ok_or_else(|| format!("MCP connection not found: {}", connection_id))?;
        session.call_tool(tool_name, arguments).await
    }

    /// Verify a server is reachable without registering it.
    pub async fn probe(config: &ConnectionConfig) -> anyhow::Result<usize> {
        let session = Arc::new(Session::new(config.clone()));
        session.connect().await.map_err(|e| anyhow::anyhow!(e))?;
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
        env: HashMap::new(),
        url: value
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        headers,
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
}
