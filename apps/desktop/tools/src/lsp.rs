//! Minimal LSP client used by the `lsp` tool.
//!
//! The `symbols` tool is regex-based and can answer "where is this definition"
//! for common languages, but it cannot answer "is this file type-correct" —
//! that needs the compiler's own analysis. This module owns one language-server
//! process per (workspace, language), speaks enough JSON-RPC to run
//! `initialize`, `textDocument/didOpen`, `textDocument/diagnostic` (pull) and
//! `textDocument/definition`, and shuts the process down with the app.
//!
//! The server is optional. A machine without `rust-analyzer` / `tsc` / `pyright`
//! gets an actionable error from the tool, not a failed Run.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex, Notify};
use tokio::time::{timeout, Instant};

use crate::{Tool, ToolContext, ToolResult};

const INIT_TIMEOUT: Duration = Duration::from_secs(25);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const DIAGNOSTIC_TIMEOUT_MS: u64 = 8_000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// A language server is a whole compiler process. Keep it for a short working
/// session, then let the next call start a fresh one instead of holding RAM
/// for the rest of the day.
const SESSION_IDLE: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Lang {
    Rust,
    TypeScript,
    Python,
    Go,
}

impl Lang {
    fn key(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::Python => "python",
            Lang::Go => "go",
        }
    }

    fn language_id(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::Python => "python",
            Lang::Go => "go",
        }
    }

    fn env_override(self) -> &'static str {
        match self {
            Lang::Rust => "MIX_LSP_RUST",
            Lang::TypeScript => "MIX_LSP_TYPESCRIPT",
            Lang::Python => "MIX_LSP_PYTHON",
            Lang::Go => "MIX_LSP_GO",
        }
    }

    fn candidates(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Lang::Rust => &[("rust-analyzer", &[])],
            Lang::TypeScript => &[("typescript-language-server", &["--stdio"])],
            Lang::Python => &[
                ("pyright-langserver", &["--stdio"]),
                ("pylsp", &["--stdio"]),
            ],
            Lang::Go => &[("gopls", &[])],
        }
    }
}

fn language_for_path(path: &Path) -> Option<Lang> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("rs") => Some(Lang::Rust),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") => Some(Lang::TypeScript),
        Some("py" | "pyi") => Some(Lang::Python),
        Some("go") => Some(Lang::Go),
        _ => None,
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(&path) {
        for ext in extensions {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_server(lang: Lang) -> Option<(PathBuf, Vec<String>)> {
    if let Ok(raw) = std::env::var(lang.env_override()) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_file() {
                return Some((path, Vec::new()));
            }
        }
    }
    for (name, args) in lang.candidates() {
        if let Some(path) = find_in_path(name) {
            return Some((path, args.iter().map(|arg| arg.to_string()).collect()));
        }
    }
    None
}

fn spawn_command(program: &Path, args: &[String], root: &Path) -> Command {
    let mut command = if cfg!(windows)
        && matches!(
            program
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase())
                .as_deref(),
            Some("cmd" | "bat")
        ) {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(program);
        command
    } else {
        Command::new(program)
    };
    command
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

type RpcResult = Result<Value, RpcError>;
type DiagnosticsMap = HashMap<String, (u64, Vec<Value>)>;
type DiagnosticsHandle = Arc<Mutex<DiagnosticsMap>>;
type PendingMap = HashMap<i64, oneshot::Sender<RpcResult>>;

async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    stdin: &Mutex<W>,
    value: &Value,
) -> anyhow::Result<()> {
    let body = serde_json::to_vec(value)?;
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    stdin.write_all(&body).await?;
    stdin.flush().await?;
    Ok(())
}

struct Session {
    lang: Lang,
    stdin: Arc<Mutex<ChildStdin>>,
    /// Kept alive for the lifetime of the session; `kill_on_drop` cleans up.
    _child: Mutex<Child>,
    pending: Arc<Mutex<PendingMap>>,
    next_id: AtomicI64,
    /// `uri -> (sequence, diagnostics)`.
    diagnostics: DiagnosticsHandle,
    diagnostic_seq: Arc<AtomicU64>,
    diagnostic_notify: Arc<Notify>,
    opened: Mutex<HashMap<String, i64>>,
    /// Serialises requests: diagnostics and definition must not interleave
    /// `didChange` notifications with a request for the previous version.
    request_lock: Mutex<()>,
    /// Last time this session served a tool call, for idle eviction.
    last_used: Mutex<Instant>,
}

impl Session {
    async fn touch(&self) {
        *self.last_used.lock().await = Instant::now();
    }
}

impl Session {
    async fn start(root: &Path, lang: Lang) -> anyhow::Result<Arc<Self>> {
        let (program, args) = resolve_server(lang).ok_or_else(|| {
            anyhow::anyhow!(
                "no language server for {} was found in PATH (expected {})",
                lang.key(),
                lang.candidates()
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(" or ")
            )
        })?;
        let mut command = spawn_command(&program, &args, root);
        let mut child = command
            .spawn()
            .map_err(|err| anyhow::anyhow!("could not start {}: {err}", program.display()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("language server stdin is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("language server stdout is unavailable"))?;
        // Keep the last few stderr lines: a rustup proxy without the
        // `rust-analyzer` component exits immediately, and "language server
        // exited" alone does not tell the user how to fix it.
        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "mixd_lsp", line = %line, "language server stderr");
                    let mut tail = tail.lock().await;
                    if tail.len() >= 12 {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            });
        }

        let stdin = Arc::new(Mutex::new(stdin));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics = Arc::new(Mutex::new(HashMap::new()));
        let seq = Arc::new(AtomicU64::new(0));
        let notify = Arc::new(Notify::new());
        tokio::spawn(read_loop(
            stdout,
            stdin.clone(),
            pending.clone(),
            diagnostics.clone(),
            seq.clone(),
            notify.clone(),
        ));

        let session = Arc::new(Self {
            lang,
            stdin,
            _child: Mutex::new(child),
            pending,
            next_id: AtomicI64::new(1),
            diagnostics,
            diagnostic_seq: seq,
            diagnostic_notify: notify,
            opened: Mutex::new(HashMap::new()),
            request_lock: Mutex::new(()),
            last_used: Mutex::new(Instant::now()),
        });

        let root_uri = url::Url::from_directory_path(root)
            .map_err(|_| anyhow::anyhow!("invalid workspace path: {}", root.display()))?
            .to_string();
        let init = session
            .request(
                "initialize",
                json!({
                    "processId": Value::Null,
                    "clientInfo": { "name": "mix-agent-desktop", "version": env!("CARGO_PKG_VERSION") },
                    "rootUri": root_uri,
                    "workspaceFolders": [{ "uri": root_uri, "name": "workspace" }],
                    // rust-analyzer runs `cargo check`, build scripts and
                    // proc-macros by default — arbitrary code from the opened
                    // repository. The tool is approval-gated too, but disable
                    // the automatic execution paths as defence in depth.
                    "initializationOptions": if lang == Lang::Rust {
                        json!({
                            "cargo": {
                                "buildScripts": { "enable": false },
                                "checkOnSave": false
                            },
                            "procMacro": { "enable": false }
                        })
                    } else {
                        json!({})
                    },
                    "capabilities": {
                        "textDocument": {
                            "definition": { "linkSupport": true },
                            "publishDiagnostics": { "relatedInformation": true },
                            "synchronization": { "dynamicRegistration": false }
                        },
                        "workspace": {
                            "workspaceFolders": true,
                            "configuration": true,
                            "workspaceEdit": { "documentChanges": true }
                        }
                    }
                }),
                INIT_TIMEOUT,
            )
            .await;
        let init = match init {
            Ok(init) => init,
            Err(err) => {
                let tail = stderr_tail.lock().await;
                let detail = if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {}", tail.iter().cloned().collect::<Vec<_>>().join(" | "))
                };
                return Err(anyhow::anyhow!(
                    "{} initialize failed: {}{detail}",
                    program.display(),
                    err.message
                ));
            }
        };
        let _ = init;
        session.notify("initialized", json!({})).await?;
        Ok(session)
    }

    async fn write_message(&self, value: &Value) -> anyhow::Result<()> {
        write_frame(&self.stdin, value).await
    }

    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn request(&self, method: &str, params: Value, wait: Duration) -> RpcResult {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let sent = self
            .write_message(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .await;
        if let Err(err) = sent {
            self.pending.lock().await.remove(&id);
            return Err(RpcError {
                code: 0,
                message: err.to_string(),
            });
        }
        match timeout(wait, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(RpcError {
                code: 0,
                message: "language server closed the connection".to_string(),
            }),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(RpcError {
                    code: 0,
                    message: format!("{method} timed out"),
                })
            }
        }
    }

    async fn open_or_change(&self, path: &Path, uri: &str) -> anyhow::Result<()> {
        let metadata = tokio::fs::metadata(path).await?;
        if metadata.len() > MAX_FILE_BYTES {
            anyhow::bail!(
                "{} is {} bytes, over the {} byte language-server limit",
                path.display(),
                metadata.len(),
                MAX_FILE_BYTES
            );
        }
        let text = tokio::fs::read_to_string(path).await?;
        let mut opened = self.opened.lock().await;
        match opened.get(uri) {
            Some(version) => {
                let next = version + 1;
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": next },
                        "contentChanges": [{ "text": text }]
                    }),
                )
                .await?;
                opened.insert(uri.to_string(), next);
                next
            }
            None => {
                self.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": self.lang.language_id(),
                            "version": 1,
                            "text": text
                        }
                    }),
                )
                .await?;
                opened.insert(uri.to_string(), 1);
                1
            }
        };
        Ok(())
    }

    async fn diagnostics(
        &self,
        path: &Path,
        uri: &str,
        wait_ms: u64,
    ) -> anyhow::Result<Vec<Value>> {
        let _guard = self.request_lock.lock().await;
        let before = self.diagnostic_seq.load(Ordering::SeqCst);
        self.open_or_change(path, uri).await?;

        // Pull diagnostics (LSP 3.17). Servers that do not implement it return
        // MethodNotFound, which is the signal to wait for publishDiagnostics.
        match self
            .request(
                "textDocument/diagnostic",
                json!({ "textDocument": { "uri": uri } }),
                Duration::from_millis(wait_ms.min(REQUEST_TIMEOUT.as_millis() as u64)),
            )
            .await
        {
            Ok(result) => {
                if result.get("kind").and_then(Value::as_str) == Some("unchanged") {
                    return Ok(self
                        .diagnostics
                        .lock()
                        .await
                        .get(uri)
                        .map(|(_, items)| items.clone())
                        .unwrap_or_default());
                }
                let items = result
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .or_else(|| result.as_array().cloned())
                    .unwrap_or_default();
                // An immediate empty pull often means the server has not
                // finished loading the crate yet (rust-analyzer does this).
                // Fall through to the push wait instead of reporting "clean".
                if !items.is_empty() {
                    return Ok(items);
                }
            }
            Err(err) if err.code == -32601 => {}
            Err(_) => {}
        }

        // Push diagnostics: wait for a newer notification for this URI. Start
        // from `before` so a cached result from a previous request is not
        // mistaken for the new one.
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        loop {
            let notified = self.diagnostic_notify.notified();
            if let Some((seq, items)) = self.diagnostics.lock().await.get(uri).cloned() {
                if seq > before {
                    return Ok(items);
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(self
                    .diagnostics
                    .lock()
                    .await
                    .get(uri)
                    .map(|(_, items)| items.clone())
                    .unwrap_or_default());
            }
            if timeout(remaining, notified).await.is_err() {
                return Ok(self
                    .diagnostics
                    .lock()
                    .await
                    .get(uri)
                    .map(|(_, items)| items.clone())
                    .unwrap_or_default());
            }
        }
    }

    async fn definition(
        &self,
        path: &Path,
        uri: &str,
        line: u32,
        character: u32,
    ) -> anyhow::Result<Value> {
        let _guard = self.request_lock.lock().await;
        self.open_or_change(path, uri).await?;
        let result = self
            .request(
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line.saturating_sub(1), "character": character.saturating_sub(1) }
                }),
                REQUEST_TIMEOUT,
            )
            .await
            .map_err(|err| anyhow::anyhow!("definition failed: {}", err.message))?;
        Ok(result)
    }
}

async fn read_loop<R, W>(
    stdout: R,
    stdin: Arc<Mutex<W>>,
    pending: Arc<Mutex<PendingMap>>,
    diagnostics: DiagnosticsHandle,
    seq: Arc<AtomicU64>,
    notify: Arc<Notify>,
) where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => {
                    let mut pending = pending.lock().await;
                    for (_, sender) in pending.drain() {
                        let _ = sender.send(Err(RpcError {
                            code: 0,
                            message: "language server exited".to_string(),
                        }));
                    }
                    notify.notify_waiters();
                    return;
                }
                Ok(_) => {}
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(raw) = trimmed.strip_prefix("Content-Length:") {
                content_length = raw.trim().parse().ok();
            }
        }
        let Some(length) = content_length else {
            continue;
        };
        let mut body = vec![0u8; length];
        if reader.read_exact(&mut body).await.is_err() {
            return;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };

        if let Some(id) = message.get("id").and_then(Value::as_i64) {
            if message.get("method").is_some() {
                // Server-to-client request. We implement none of the optional
                // features, so answer with a null result. Without a reply the
                // server can block waiting for us (notably workspace/configuration).
                let result = if message.get("method").and_then(Value::as_str)
                    == Some("workspace/configuration")
                {
                    let count = message
                        .get("params")
                        .and_then(|p| p.get("items"))
                        .and_then(Value::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0);
                    Value::Array(vec![Value::Null; count])
                } else {
                    Value::Null
                };
                let body = serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                }))
                .unwrap_or_default();
                let mut stdin = stdin.lock().await;
                let _ = stdin
                    .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
                    .await;
                let _ = stdin.write_all(&body).await;
                let _ = stdin.flush().await;
            } else {
                let sender = pending.lock().await.remove(&id);
                if let Some(sender) = sender {
                    let result = match message.get("error") {
                        Some(error) => Err(RpcError {
                            code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                            message: error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown error")
                                .to_string(),
                        }),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = sender.send(result);
                }
            }
            continue;
        }

        if message.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        {
            let Some(params) = message.get("params") else {
                continue;
            };
            let Some(uri) = params.get("uri").and_then(Value::as_str) else {
                continue;
            };
            let items = params
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let sequence = seq.fetch_add(1, Ordering::SeqCst) + 1;
            diagnostics
                .lock()
                .await
                .insert(uri.to_string(), (sequence, items));
            notify.notify_waiters();
        }
    }
}

pub struct LspTool {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl Default for LspTool {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl LspTool {
    async fn session(&self, root: &Path, lang: Lang) -> anyhow::Result<Arc<Session>> {
        let key = format!("{}|{}", root.display(), lang.key());
        {
            let sessions = self.sessions.lock().await;
            if let Some(session) = sessions.get(&key).cloned() {
                drop(sessions);
                session.touch().await;
                return Ok(session);
            }
        }

        // Evict sessions that have not served a call recently. Dropping the
        // `Arc` is what shuts the server down (`kill_on_drop` on the child).
        let stale: Vec<String> = {
            let sessions = self.sessions.lock().await;
            let mut stale = Vec::new();
            for (key, session) in sessions.iter() {
                if session.last_used.lock().await.elapsed() >= SESSION_IDLE {
                    stale.push(key.clone());
                }
            }
            stale
        };
        if !stale.is_empty() {
            let mut sessions = self.sessions.lock().await;
            for key in stale {
                sessions.remove(&key);
            }
        }

        let session = Session::start(root, lang).await?;
        self.sessions.lock().await.insert(key, session.clone());
        Ok(session)
    }

    fn normalize_diagnostics(path: &Path, items: Vec<Value>) -> Vec<Value> {
        items
            .into_iter()
            .take(200)
            .map(|item| {
                let severity = match item.get("severity").and_then(Value::as_i64) {
                    Some(1) => "error",
                    Some(2) => "warning",
                    Some(3) => "info",
                    _ => "hint",
                };
                json!({
                    "path": path.to_string_lossy(),
                    "line": item.pointer("/range/start/line").and_then(Value::as_u64).map(|v| v + 1),
                    "column": item.pointer("/range/start/character").and_then(Value::as_u64).map(|v| v + 1),
                    "endLine": item.pointer("/range/end/line").and_then(Value::as_u64).map(|v| v + 1),
                    "endColumn": item.pointer("/range/end/character").and_then(Value::as_u64).map(|v| v + 1),
                    "severity": severity,
                    "message": item.get("message").cloned().unwrap_or(Value::Null),
                    "source": item.get("source").cloned().unwrap_or(Value::Null),
                    "code": item.get("code").cloned().unwrap_or(Value::Null),
                })
            })
            .collect()
    }

    fn normalize_locations(result: &Value) -> Vec<Value> {
        let candidates = result.as_array().cloned().unwrap_or_default();
        candidates
            .into_iter()
            .take(50)
            .filter_map(|item| {
                let uri = item
                    .get("uri")
                    .or_else(|| item.get("targetUri"))
                    .and_then(Value::as_str)?;
                let range = item
                    .get("range")
                    .or_else(|| item.get("targetSelectionRange"))
                    .or_else(|| item.get("targetRange"))?;
                let line = range.pointer("/start/line").and_then(Value::as_u64)? + 1;
                let column = range.pointer("/start/character").and_then(Value::as_u64)? + 1;
                let path = url::Url::parse(uri)
                    .ok()
                    .and_then(|url| url.to_file_path().ok())
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| uri.to_string());
                Some(json!({ "path": path, "line": line, "column": column }))
            })
            .collect()
    }
}

#[async_trait]
impl Tool for LspTool {
    fn id(&self) -> &str {
        "lsp"
    }
    fn name(&self) -> &str {
        "Language Server"
    }
    fn description(&self) -> &str {
        "Ask the real language server for compiler diagnostics or a definition. Use \
         action \"diagnostics\" after editing a source file to see type errors before \
         running a full build, and action \"definition\" to jump to a symbol's \
         declaration. Requires the language server to be installed (rust-analyzer, \
         typescript-language-server, pyright/pylsp, or gopls)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["diagnostics", "definition"],
                    "default": "diagnostics"
                },
                "path": { "type": "string", "description": "Workspace-relative source file." },
                "line": { "type": "integer", "description": "1-based line for definition.", "default": 1 },
                "column": { "type": "integer", "description": "1-based column for definition.", "default": 1 },
                "wait_ms": { "type": "integer", "description": "How long to wait for push diagnostics, capped at 15000 ms.", "default": 8000 }
            },
            "required": ["action", "path"]
        })
    }
    fn is_sensitive(&self, _args: &Value) -> bool {
        // Starting a language server executes code from the opened repository
        // (cargo check / build scripts / proc macros) and inherits the user's
        // environment. Never dispatch it without an approval decision.
        true
    }
    fn pattern_target(&self, args: &Value) -> Option<String> {
        args.get("path").and_then(Value::as_str).map(str::to_string)
    }

    async fn invoke(&self, ctx: ToolContext, args: Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("diagnostics");
        let raw_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("path is required"))?;
        let root = match ctx.root_path.as_deref() {
            Some(root) => PathBuf::from(root),
            None => std::env::current_dir()?,
        };
        let root = root.canonicalize().unwrap_or(root);
        let path = crate::files::FilesTool::safe_join(&root, raw_path)?;
        if !path.is_file() {
            return Ok(ToolResult::fail(format!("not a file: {}", path.display())));
        }
        let Some(lang) = language_for_path(&path) else {
            return Ok(ToolResult {
                ok: false,
                content: json!({ "path": path.to_string_lossy(), "supported": ["rust", "typescript", "python", "go"] }),
                error: Some(format!(
                    "no language server is configured for {}",
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("this file")
                )),
            });
        };
        let uri = url::Url::from_file_path(&path)
            .map_err(|_| anyhow::anyhow!("could not build a file URI for {}", path.display()))?
            .to_string();

        let session = match self.session(&root, lang).await {
            Ok(session) => session,
            Err(err) => {
                return Ok(ToolResult {
                    ok: false,
                    content: json!({ "language": lang.key(), "path": path.to_string_lossy() }),
                    error: Some(err.to_string()),
                });
            }
        };

        match action {
            "definition" => {
                let line = args.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
                let column = args.get("column").and_then(Value::as_u64).unwrap_or(1) as u32;
                let result = session.definition(&path, &uri, line, column).await?;
                let locations = Self::normalize_locations(&result);
                Ok(ToolResult {
                    ok: true,
                    content: json!({
                        "action": "definition",
                        "path": path.to_string_lossy(),
                        "language": lang.key(),
                        "locations": locations,
                        "count": locations.len(),
                    }),
                    error: None,
                })
            }
            _ => {
                let wait_ms = args
                    .get("wait_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(DIAGNOSTIC_TIMEOUT_MS)
                    .clamp(500, 15_000);
                let items = session.diagnostics(&path, &uri, wait_ms).await?;
                let diagnostics = Self::normalize_diagnostics(&path, items);
                Ok(ToolResult {
                    ok: true,
                    content: json!({
                        "action": "diagnostics",
                        "path": path.to_string_lossy(),
                        "language": lang.key(),
                        "diagnostics": diagnostics,
                        "count": diagnostics.len(),
                    }),
                    error: None,
                })
            }
        }
    }

    async fn idle_sweep(&self) {
        let stale: Vec<String> = {
            let sessions = self.sessions.lock().await;
            let mut stale = Vec::new();
            for (key, session) in sessions.iter() {
                if session.last_used.lock().await.elapsed() >= SESSION_IDLE {
                    stale.push(key.clone());
                }
            }
            stale
        };
        if stale.is_empty() {
            return;
        }
        let mut sessions = self.sessions.lock().await;
        for key in stale {
            if sessions.remove(&key).is_some() {
                tracing::info!(session = %key, "stopping an idle language server");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncBufRead, AsyncBufReadExt, BufReader};

    async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Value {
        let mut length = None;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("header");
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let mut body = vec![0u8; length.expect("length")];
        reader.read_exact(&mut body).await.expect("body");
        serde_json::from_slice(&body).expect("json")
    }

    #[tokio::test]
    async fn read_loop_routes_responses_and_diagnostics() {
        let (client_write, server_read) = duplex(16 * 1024);
        let (server_write, client_read) = duplex(16 * 1024);
        let stdin = Arc::new(Mutex::new(client_write));
        let pending: Arc<Mutex<PendingMap>> = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: DiagnosticsHandle = Arc::new(Mutex::new(HashMap::new()));
        let seq = Arc::new(AtomicU64::new(0));
        let notify = Arc::new(Notify::new());
        let reader = tokio::spawn(read_loop(
            client_read,
            stdin.clone(),
            pending.clone(),
            diagnostics.clone(),
            seq.clone(),
            notify.clone(),
        ));

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_read);
            let request = read_frame(&mut reader).await;
            assert_eq!(request["id"], 1);
            let writer = Mutex::new(server_write);
            write_frame(
                &writer,
                &json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }),
            )
            .await
            .expect("response");
            write_frame(
                &writer,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": { "uri": "file:///tmp/a.rs", "diagnostics": [{ "message": "boom" }] }
                }),
            )
            .await
            .expect("diagnostic");
        });

        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(1, tx);
        write_frame(
            &stdin,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        )
        .await
        .expect("request");
        assert_eq!(rx.await.expect("reply").expect("ok")["ok"], true);

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let notified = notify.notified();
            if diagnostics.lock().await.contains_key("file:///tmp/a.rs") {
                break;
            }
            if timeout(deadline.saturating_duration_since(Instant::now()), notified)
                .await
                .is_err()
            {
                panic!("diagnostic notification was not routed");
            }
        }
        assert_eq!(
            diagnostics.lock().await["file:///tmp/a.rs"].1[0]["message"],
            "boom"
        );
        server.await.expect("server");
        reader.abort();
    }

    #[test]
    fn extensions_map_to_a_language() {
        assert_eq!(language_for_path(Path::new("a.rs")), Some(Lang::Rust));
        assert_eq!(
            language_for_path(Path::new("a.tsx")),
            Some(Lang::TypeScript)
        );
        assert_eq!(language_for_path(Path::new("a.py")), Some(Lang::Python));
        assert_eq!(language_for_path(Path::new("a.go")), Some(Lang::Go));
        assert_eq!(language_for_path(Path::new("a.txt")), None);
    }

    #[test]
    fn definition_locations_are_one_based() {
        let result = json!([{
            "uri": "file:///tmp/a.rs",
            "range": { "start": { "line": 4, "character": 8 }, "end": { "line": 4, "character": 10 } }
        }]);
        let locations = LspTool::normalize_locations(&result);
        assert_eq!(locations[0]["line"], 5);
        assert_eq!(locations[0]["column"], 9);
        assert!(locations[0]["path"].as_str().unwrap().ends_with("a.rs"));
    }

    #[test]
    fn location_links_are_accepted() {
        let result = json!([{
            "targetUri": "file:///tmp/b.py",
            "targetSelectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }
        }]);
        assert_eq!(LspTool::normalize_locations(&result).len(), 1);
    }

    #[test]
    fn diagnostics_are_normalized_with_severity() {
        let items = vec![json!({
            "range": { "start": { "line": 2, "character": 1 }, "end": { "line": 2, "character": 4 } },
            "severity": 1,
            "message": "expected `;`",
            "source": "rust-analyzer"
        })];
        let out = LspTool::normalize_diagnostics(Path::new("a.rs"), items);
        assert_eq!(out[0]["severity"], "error");
        assert_eq!(out[0]["line"], 3);
    }
}
