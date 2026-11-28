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
use std::net::{IpAddr, SocketAddr};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{timeout, Duration};

use crate::error::McpError;

/// Keep a bounded, de-identified excerpt of a stdio server's stderr line.
///
/// OAuth failures and debug logs routinely echo `Authorization` values or
/// bearer tokens. Logging them verbatim (even at `debug`) copied credentials
/// into the app log, so mask `name=value` / `name: value` pairs whose name
/// looks sensitive and cap the length.
fn redact_stderr_line(line: &str) -> String {
    const MAX: usize = 512;
    let truncated: String = line.chars().take(MAX).collect();
    let lowered = truncated.to_ascii_lowercase();
    let sensitive = [
        "authorization",
        "bearer ",
        "token",
        "secret",
        "password",
        "api_key",
        "apikey",
        "client_secret",
    ];
    if sensitive.iter().any(|key| lowered.contains(key)) {
        // Do not try to surgically mask: offsets drift as values are replaced,
        // and a partial mask can still leak the tail of a credential.
        format!(
            "[redacted: {} bytes, contained a credential-shaped key]",
            line.len()
        )
    } else {
        truncated
    }
}

/// MCP protocol revision this client advertises.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Default overall deadline for one Streamable-HTTP JSON-RPC exchange, headers
/// and body together. A server that trickles bytes forever would otherwise hold
/// a `tools/call` open indefinitely, since the shared HTTP client only bounds
/// the gap between reads. Generous enough for genuinely slow tool work, and
/// overridable per connection via [`ConnectionConfig::http_timeout_secs`].
pub const DEFAULT_HTTP_CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Hard cap on one unterminated SSE line from an MCP server.
///
/// The shared HTTP client only bounds the gap between reads and this crate
/// allows a 300 s overall exchange, so a peer that streams bytes forever
/// without a newline could otherwise grow the line buffer into the gigabytes.
/// Mirrors `mixd_providers::transport::MAX_SSE_BUFFER_BYTES`; no legitimate
/// JSON-RPC frame is anywhere near 16 MiB.
pub const MAX_SSE_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Append one network chunk to the SSE line buffer and return every complete
/// `data:` payload. An unterminated line larger than [`MAX_SSE_BUFFER_BYTES`]
/// fails the exchange instead of growing without bound.
fn append_sse_buffer(buffer: &mut String, bytes: &[u8]) -> Result<Vec<String>, McpError> {
    buffer.push_str(&String::from_utf8_lossy(bytes));
    // Check *before* draining complete lines: a single chunk ending in a
    // newline used to bypass the cap entirely and hand a >16 MiB payload to
    // the JSON parser.
    if buffer.len() > MAX_SSE_BUFFER_BYTES {
        let len = buffer.len();
        buffer.clear();
        return Err(McpError::SseBufferOverflow(len));
    }
    let mut payloads = Vec::new();
    while let Some(idx) = buffer.find('\n') {
        let line: String = buffer.drain(..=idx).collect();
        if let Some(payload) = line.trim().strip_prefix("data:") {
            payloads.push(payload.trim().to_string());
        }
    }
    Ok(payloads)
}

/// Resolve an HTTP endpoint to addresses that are safe to dial.
///
/// Literal IPs are validated synchronously; hostnames are resolved once and
/// every answer is gated, including round-robin mixes of public and private
/// addresses. The returned addresses are used to *pin* the HTTP client so the
/// DNS lookup that reqwest performs later cannot resolve to a different IP.
pub(crate) async fn resolve_allowed_endpoint(
    url: &str,
) -> Result<(String, Vec<SocketAddr>), McpError> {
    let parsed = url::Url::parse(url).map_err(|err| McpError::InvalidUrl(url.to_string(), err))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(McpError::PrivateEndpoint(format!(
                "unsupported scheme `{other}`"
            )))
        }
    }
    let Some(host) = parsed.host_str() else {
        return Err(McpError::PrivateEndpoint("missing host".to_string()));
    };
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        if mixd_browser::is_forbidden_ip(&ip, false) {
            return Err(McpError::PrivateEndpoint(ip.to_string()));
        }
        // No DNS involved for a literal IP; nothing to pin.
        return Ok((host.to_string(), Vec::new()));
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let lookup = timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((bare, port)),
    )
    .await
    .map_err(|_| McpError::PrivateEndpoint(format!("DNS lookup timed out for {bare}")))?
    .map_err(|err| McpError::PrivateEndpoint(format!("DNS lookup failed for {bare}: {err}")))?;
    let addrs: Vec<SocketAddr> = lookup.collect();
    if addrs.is_empty() {
        return Err(McpError::PrivateEndpoint(format!(
            "DNS returned no addresses for {bare}"
        )));
    }
    for addr in &addrs {
        if mixd_browser::is_forbidden_ip(&addr.ip(), false) {
            return Err(McpError::PrivateEndpoint(addr.ip().to_string()));
        }
    }
    Ok((bare.to_string(), addrs))
}

/// An HTTP client whose DNS answers are pinned and whose redirects are off.
///
/// Two SSRF shapes remained after `reject_private_endpoint`: a DNS change
/// between our lookup and reqwest's own resolution, and a 3xx redirect to a
/// private address after an allowed first hop. Pinning the addresses we
/// vetted on the client and refusing redirects closes both without needing a
/// socket-level interceptor.
pub struct SafeHttpClient {
    client: reqwest::Client,
}

impl SafeHttpClient {
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

/// Build a [`SafeHttpClient`] for `url`, validating and pinning its host.
pub async fn safe_http_client_for(url: &str) -> Result<SafeHttpClient, McpError> {
    let (host, addrs) = resolve_allowed_endpoint(url).await?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(120))
        .pool_max_idle_per_host(4);
    if !addrs.is_empty() {
        builder = builder.resolve_to_addrs(&host, &addrs);
    }
    let client = builder
        .build()
        .map_err(|err| McpError::Transport(err.to_string()))?;
    Ok(SafeHttpClient { client })
}

/// Reject a configured HTTP endpoint that points at a private / link-local /
/// otherwise forbidden address *before* opening a socket.
///
/// Uses the same predicate as the browser `navigate` gate and the tools egress
/// policy (`mixd_browser::is_forbidden_ip`), so the three surfaces cannot drift
/// on what "private" means. Literal IPs are checked synchronously; hostnames
/// are resolved once and every answer is gated, including round-robin mixes of
/// public and private addresses.
///
/// Note the remaining TOCTOU: DNS can change between this lookup and the
/// resolution `reqwest` performs when dialing, and an HTTP redirect can land on
/// a private address after the first hop. Pinning the resolved addresses on the
/// request (or socket-level interception) is the durable fix; this is the cheap
/// early rejection that stops the common literal-IP SSRF shape.
#[cfg(test)]
pub(crate) async fn reject_private_endpoint(url: &str) -> Result<(), McpError> {
    resolve_allowed_endpoint(url).await.map(|_| ())
}

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
        // Reject loopback / link-local / RFC1918 literals (and hostnames that
        // resolve to them) before the configured headers — which may include a
        // bearer token — are sent anywhere. See the TOCTOU note on the helper.
        // Build a client with the vetted addresses pinned and redirects
        // disabled; the shared provider client would re-resolve DNS and follow
        // a 3xx into a private network.
        let safe = safe_http_client_for(&url).await?;
        let mut req = safe
            .client()
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
                // A peer that never terminates a line must not be able to grow
                // this buffer for the life of the 300 s exchange.
                for payload in append_sse_buffer(&mut buffer, &bytes)? {
                    if let Ok(value) = serde_json::from_str::<Value>(&payload) {
                        return Ok(value);
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
            // tokio defaults to inheriting the parent's handles. Without
            // explicit pipes `child.stdin.take()` is always `None` and every
            // stdio server failed with `NoStdin` before it could say hello;
            // stderr is piped too and drained below so a chatty server cannot
            // block on a full inherited pipe.
            let mut child = match Command::new(&command)
                .args(&self.config.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
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
            if let Some(stderr) = child.stderr.take() {
                let stderr_id = self.config.id.clone();
                tauri_free_spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        // Server logs are useful when a handshake fails, but
                        // noisy in the default log level.
                        tracing::debug!(connection = %stderr_id, "MCP server stderr: {}", redact_stderr_line(&line));
                    }
                });
            }
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
        /// One unreachable server must not hold the whole background pass.
        const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
        let sessions: Vec<Arc<Session>> = self.sessions.read().await.values().cloned().collect();
        let mut total = 0;
        for session in sessions {
            let result = match timeout(CONNECT_TIMEOUT, session.connect()).await {
                Ok(result) => result.map_err(|reason| reason.to_string()),
                Err(_) => Err("connect timed out".to_string()),
            };
            match result {
                Ok(()) => {
                    total += session.tools.read().await.len();
                }
                Err(reason) => {
                    tracing::warn!(
                        connection = %session.config.id,
                        %reason,
                        "MCP session could not be connected"
                    );
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

    #[test]
    fn stderr_logging_does_not_echo_credentials() {
        let line = "error: Authorization: Bearer super-secret-token from Authorization header";
        let redacted = redact_stderr_line(line);
        assert!(!redacted.contains("super-secret"));
        assert!(redacted.contains("[redacted"));
        assert_eq!(
            redact_stderr_line("server starting on port 1234"),
            "server starting on port 1234"
        );
        assert_eq!(redact_stderr_line(&"x".repeat(600)).len(), 512);
    }

    #[test]
    fn sse_data_lines_are_drained_across_chunks() {
        let mut buffer = String::new();
        let first = append_sse_buffer(&mut buffer, b"data: {\"jsonrpc\"").expect("under the cap");
        assert!(first.is_empty(), "a partial line is not an event");
        let second = append_sse_buffer(&mut buffer, b":\"2.0\"}\n: keepalive\ndata: [DONE]\n")
            .expect("under the cap");
        assert_eq!(
            second,
            vec!["{\"jsonrpc\":\"2.0\"}".to_string(), "[DONE]".to_string()]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn an_unterminated_sse_line_is_capped() {
        // A server streaming bytes with no newline must not grow the buffer
        // past the cap while the 300 s exchange deadline runs.
        let mut buffer = String::new();
        let chunk = vec![b'x'; MAX_SSE_BUFFER_BYTES + 1];
        let err = append_sse_buffer(&mut buffer, &chunk).expect_err("over the cap");
        assert!(
            matches!(err, McpError::SseBufferOverflow(_)),
            "unexpected: {err:?}"
        );
        assert!(buffer.is_empty(), "the rejected buffer must be released");
    }

    #[tokio::test]
    async fn private_http_endpoints_are_rejected_before_dialing() {
        for url in [
            "http://127.0.0.1/",
            "http://127.0.0.1:1234/mcp",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://10.0.0.5/mcp",
            "http://192.168.1.1/mcp",
        ] {
            let err = reject_private_endpoint(url).await.expect_err("private IP");
            assert!(
                matches!(err, McpError::PrivateEndpoint(_)),
                "{url} was not rejected as private: {err:?}"
            );
        }
        // A non-HTTP endpoint cannot be POSTed to either.
        assert!(reject_private_endpoint("file:///etc/passwd").await.is_err());
    }

    #[tokio::test]
    async fn a_remote_connection_to_a_private_ip_fails_without_a_request() {
        let cfg = ConnectionConfig {
            id: "ssrf".into(),
            name: "ssrf".into(),
            transport: Transport::StreamableHttp,
            command: None,
            args: Vec::new(),
            url: Some("http://169.254.169.254/latest/meta-data/".into()),
            headers: Default::default(),
            http_timeout_secs: None,
        };
        let session = Session::new(cfg);
        let err = session.connect().await.expect_err("no request is sent");
        assert!(
            matches!(err, McpError::PrivateEndpoint(_)),
            "unexpected: {err:?}"
        );
    }
}
