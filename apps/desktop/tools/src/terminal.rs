//! Persistent shell for the agent.
//!
//! The previous implementation spawned a fresh process per call, which meant
//! `cd build` did nothing, `export FOO=1` did not survive, and interactive tools
//! behaved differently than they do for a human. `portable-pty` was declared as
//! a dependency and never used.
//!
//! This implementation keeps one PTY per workspace, driven by a real login
//! shell. Commands are delimited with an unguessable sentinel and the exit code
//! is read back out of a status file, which is the only reliable way to get a
//! shell's exit status over a PTY — the shell does not report it on the wire.
//!
//! The one-shot path is kept as a fallback for the case where a PTY cannot be
//! spawned at all, so a missing PTY degrades to slightly worse behaviour rather
//! than to a broken tool.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::Mutex;

use super::{Tool, ToolContext, ToolResult};

mod parsing;
mod sandbox;
mod session;
mod shell;
#[cfg(test)]
mod tests;

use parsing::*;
use sandbox::*;
use shell::*;

const DEFAULT_MAX_WALL_CLOCK_MS: u64 = 30_000;
/// Hard ceiling on a single `terminal` call. Without it the model could ask for
/// `max_wall_clock_ms: 999999999` and park the Run (and its cancellation) for
/// days.
const MAX_WALL_CLOCK_MS: u64 = 600_000;
/// How much output to return. A runaway `yes` must not fill the context window.
const MAX_OUTPUT_BYTES: usize = 200 * 1024;
/// How many completed commands to retain. Each record can hold `MAX_OUTPUT_BYTES`
/// of output, so an unbounded history is an unbounded memory leak over a long
/// session.
const MAX_HISTORY: usize = 200;

pub struct TerminalTool {
    pub max_wall_clock_ms: u64,
    pub history: Arc<Mutex<Vec<TerminalCommand>>>,
    /// Live shells, keyed per (Run, directory) so concurrent Runs and projects
    /// do not share a `cd` or environment.
    sessions: Arc<Mutex<HashMap<String, Arc<ShellSession>>>>,
    /// When set, a new shell is started under the platform sandbox (macOS:
    /// `sandbox-exec` with writes confined to the workspace and temp dirs).
    /// Shared so a settings change reaches the registry's clone.
    sandbox: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for TerminalTool {
    fn default() -> Self {
        Self {
            max_wall_clock_ms: DEFAULT_MAX_WALL_CLOCK_MS,
            history: Arc::new(Mutex::new(Vec::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            sandbox: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl TerminalTool {
    /// Turn the OS sandbox on or off. Takes effect for shells spawned after
    /// this call; an already-running shell keeps the profile it started with.
    pub fn set_sandbox(&self, enabled: bool) {
        self.sandbox
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn sandbox_enabled(&self) -> bool {
        self.sandbox.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalCommand {
    pub command: String,
    pub args: Vec<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// The working directory the command ran in, after any `cd`.
    pub cwd: Option<String>,
    /// True when the command ran in the persistent shell rather than a
    /// one-shot child process.
    pub persistent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellLaunchConfig {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub is_posix: bool,
}

/// Detect the best available shell for the current platform (OpenCode style).
///
/// On Windows, Git Bash (or MSYS2/Scoop) is preferred so the LLM gets a consistent
/// POSIX environment (`ls`, `cat`, `grep`, `git`, etc.) rather than PowerShell syntax.
/// On macOS and Linux, the user's `$SHELL` or standard POSIX shells are used.
/// Can be overridden via the `MIX_SHELL` environment variable.
struct ShellSession {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Everything the reader thread has seen so far.
    output: Arc<Mutex<Vec<u8>>>,
    /// The working directory the shell is in, tracked by echoing `pwd`.
    cwd: Arc<Mutex<String>>,
    id: String,
    /// Kill handle for the shell process. The reader thread owns the `Child`
    /// (it is the one calling `wait`), so the killer is cloned out of it and
    /// kept here: without it a timed-out command kept running for the rest of
    /// the session.
    killer: std::sync::Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    config: ShellLaunchConfig,
}

pub fn truncate_to_char_boundary(text: &str, max_bytes: usize) -> &str {
    let mut cut = max_bytes.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
}

#[async_trait]
impl Tool for TerminalTool {
    fn id(&self) -> &str {
        "terminal"
    }
    fn name(&self) -> &str {
        "Terminal"
    }
    fn description(&self) -> &str {
        "Run a shell command in the workspace. The shell is persistent: `cd`, environment \
         variables and shell state carry over between calls, so `cd src` followed by \
         `ls` works. Prefer `command` as a single string (`cargo test --workspace`) rather \
         than `args`; a bare executable with no shell features can use `args`."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command, interpreted by the workspace shell."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional argv for a direct exec, bypassing the shell. `command` must be an executable path."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional subdirectory of the workspace to run in. Must stay inside the workspace root; anything outside is rejected."
                },
                "max_wall_clock_ms": { "type": "integer", "default": 30000 }
            },
            "required": ["command"]
        })
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("command required"))?
            .to_string();
        let cmd_args: Vec<String> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let max_ms = args
            .get("max_wall_clock_ms")
            .and_then(|v| v.as_u64())
            .filter(|ms| *ms > 0)
            .unwrap_or(if self.max_wall_clock_ms > 0 {
                self.max_wall_clock_ms
            } else {
                DEFAULT_MAX_WALL_CLOCK_MS
            })
            .min(MAX_WALL_CLOCK_MS);
        let workdir = match ctx.root_path.as_deref() {
            Some(root) => std::path::PathBuf::from(root),
            None => std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        };
        // Fail closed: an unresolvable workspace root is an error, never a
        // lexical fallback (the old `unwrap_or(workdir)` kept going on a
        // path an attacker had just replaced).
        let workdir = match canonicalize_portable(&workdir) {
            Ok(canon) => canon,
            Err(err) => {
                return Err(anyhow::anyhow!("workspace root not found: {}", err));
            }
        };

        // An explicit `cwd` must stay inside the workspace. Without this the
        // schema advertises a free directory pick while the shell happily
        // obliges with `/etc`.
        let session_dir = match args.get("cwd").and_then(|v| v.as_str()) {
            None => workdir.clone(),
            Some(requested) if requested.trim().is_empty() => workdir.clone(),
            Some(requested) => {
                // Strict resolution: the directory must already exist and
                // canonicalize cleanly. A missing path is rejected rather
                // than joined lexically, so a symlink swapped in between the
                // check and the `cd` cannot divert the session.
                let candidate = if std::path::Path::new(requested).is_absolute() {
                    std::path::PathBuf::from(requested)
                } else {
                    workdir.join(requested.trim_start_matches('/'))
                };
                let canon = match canonicalize_portable(&candidate) {
                    Ok(canon) => canon,
                    Err(_) => {
                        return Ok(ToolResult {
                            ok: false,
                            content: serde_json::json!({
                                "command": command,
                                "exit_code": null,
                                "cwd": workdir.to_string_lossy(),
                            }),
                            error: Some(
                                "cwd does not exist or is outside the workspace".to_string(),
                            ),
                        });
                    }
                };
                if !canon.starts_with(&workdir) {
                    return Ok(ToolResult {
                        ok: false,
                        content: serde_json::json!({
                            "command": command,
                            "exit_code": null,
                            "cwd": workdir.to_string_lossy(),
                        }),
                        error: Some("cwd is outside the workspace".to_string()),
                    });
                }
                canon
            }
        };

        let started = chrono::Utc::now();
        let (stdout, exit_code, cwd, persistent) = if cmd_args.is_empty() {
            self.run_in_shell(
                &workdir,
                &session_dir,
                &ctx.run_id.to_string(),
                &command,
                max_ms,
            )
            .await
        } else if self.sandbox_enabled() {
            // `args` means a direct exec, so state must not carry over. It still
            // has to go through the sandbox, or `terminal` with `args` would be
            // the one unconfined escape hatch in an otherwise sandboxed run.
            match sandboxed_exec(&command, &cmd_args, &workdir) {
                Some((program, args)) => self.run_once(&session_dir, &program, &args, max_ms).await,
                None => {
                    return Ok(ToolResult {
                        ok: false,
                        content: serde_json::json!({
                            "command": command,
                            "exit_code": null,
                            "cwd": workdir.to_string_lossy(),
                        }),
                        error: Some("sandboxed exec unavailable on this platform".to_string()),
                    });
                }
            }
        } else {
            // `args` means a direct exec, so state must not carry over.
            self.run_once(&session_dir, &command, &cmd_args, max_ms)
                .await
        };
        let finished = chrono::Utc::now();

        let record = TerminalCommand {
            command: if cmd_args.is_empty() {
                command
            } else {
                format!("{} {}", command, cmd_args.join(" "))
            },
            args: cmd_args,
            started_at: started,
            finished_at: finished,
            exit_code,
            stdout: stdout.clone(),
            stderr: String::new(),
            cwd,
            persistent,
        };
        let ok = exit_code == Some(0);
        let payload = serde_json::to_value(&record).unwrap_or_else(|_| serde_json::json!({}));
        let err_msg = if ok {
            None
        } else {
            Some(match exit_code {
                Some(code) => format!("exit code {code}"),
                None => "command did not report an exit status".to_string(),
            })
        };
        {
            let mut history = self.history.lock().await;
            history.push(record);
            if history.len() > MAX_HISTORY {
                let excess = history.len() - MAX_HISTORY;
                history.drain(..excess);
            }
        }
        Ok(ToolResult {
            ok,
            content: payload,
            error: err_msg,
        })
    }
}

/// Keep the live-PTY map bounded. Keys are per (Run, directory), so the map
/// grows with every Run rather than with the number of projects.
fn evict_excess_shells(sessions: &mut HashMap<String, Arc<ShellSession>>, keep: &str) {
    const MAX_SHELL_SESSIONS: usize = 24;
    while sessions.len() > MAX_SHELL_SESSIONS {
        let Some(victim) = sessions.keys().find(|k| k.as_str() != keep).cloned() else {
            break;
        };
        if let Some(session) = sessions.remove(&victim) {
            // Dropping the Arc is not enough: the reader thread owns the
            // `Child` and would keep the login shell alive.
            if let Ok(mut killer) = session.killer.lock() {
                let _ = killer.kill();
            }
        }
    }
}

impl TerminalTool {
    /// Get or create the persistent shell for this workspace and run `command`.
    ///
    /// `project_root` is the workspace boundary; `session_dir` is where this
    /// particular shell lives (the root or a validated `cwd` subdirectory). A
    /// previous turn's `cd /etc` must not leak into the next command, so a
    /// session found outside the project is steered back before running.
    async fn run_in_shell(
        &self,
        project_root: &std::path::Path,
        session_dir: &std::path::Path,
        run_scope: &str,
        command: &str,
        max_ms: u64,
    ) -> (String, Option<i32>, Option<String>, bool) {
        // One shell per (Run, directory), not per directory. Two Runs in the
        // same project used to share a `cd`/env state: a delegate or a second
        // conversation in another window could silently move the parent's
        // shell, and the parent's next command ran somewhere it did not expect.
        let key = format!("{run_scope}|{}", session_dir.to_string_lossy());
        // A failed spawn must not be cached, or every later call in this
        // workspace is permanently degraded.
        let session = {
            let mut sessions = self.sessions.lock().await;
            match sessions.get(&key) {
                Some(existing) => Some(existing.clone()),
                None => {
                    // A per-session nonce: the id feeds both the sentinel and
                    // the exit-status file, and a replacement shell for the same
                    // workspace used to inherit the orphaned one's path, so a
                    // late `printf > status` overwrote the new command's code.
                    static SESSION_SEQ: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    let session_id = format!(
                        "{}-{}-{}",
                        key.len(),
                        stable_hash(&key),
                        SESSION_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    );
                    let sandbox_root = self.sandbox_enabled().then_some(session_dir);
                    let spawned = ShellSession::spawn(&key, &session_id, sandbox_root);
                    if let Some(session) = &spawned {
                        sessions.insert(key.clone(), session.clone());
                        evict_excess_shells(&mut sessions, &key);
                    }
                    spawned
                }
            }
        };
        let Some(session) = session else {
            // A sandbox was explicitly requested: degrading to an unsandboxed
            // one-shot child would quietly defeat the setting, so fail instead.
            if self.sandbox_enabled() {
                return (
                    "sandboxed shell unavailable: could not start a PTY under the sandbox"
                        .to_string(),
                    None,
                    Some(session_dir.to_string_lossy().to_string()),
                    false,
                );
            }
            // No PTY available: a one-shot child still runs the command.
            // Warn: this path loses shell state (`cd` does not persist) and
            // runs outside the sandbox-steering below, so callers must not
            // assume continuity here.
            tracing::warn!(workspace = %key, "no PTY available; running one-shot without persistent shell state");
            let shell_cfg = detect_shell();
            let (shell_cmd, shell_args) = if shell_cfg.is_posix {
                (
                    shell_cfg.program.to_string_lossy().to_string(),
                    vec!["-c".to_string(), command.to_string()],
                )
            } else {
                (
                    shell_cfg.program.to_string_lossy().to_string(),
                    vec![
                        "-NoProfile".to_string(),
                        "-NonInteractive".to_string(),
                        "-Command".to_string(),
                        command.to_string(),
                    ],
                )
            };
            let (out, code, cwd, _) = self
                .run_once(session_dir, &shell_cmd, &shell_args, max_ms)
                .await;
            return (out, code, Some(cwd.unwrap_or_else(|| {
                // Sandbox boundary: the one-shot fallback has no live `$PWD`
                // to read back, so the workspace root is the only safe answer —
                // never the process CWD, which may be outside the project.
                tracing::warn!(workspace = %key, "one-shot fallback reported no cwd; using the workspace root");
                session_dir.to_string_lossy().to_string()
            })), false);
        };
        // A previous `cd` outside the project must not persist: steer the
        // shell back before running so turn N+1 does not execute in `/etc`
        // because turn N ended there. Both sides are canonicalized so a
        // symlinked `$PWD` cannot fake containment.
        if !cwd_inside(&session.cwd().await, project_root) {
            let home = if session.config.is_posix {
                posix_path_expression(session_dir)
            } else {
                powershell_quote(&session_dir.to_string_lossy())
            };
            let cd_cmd = if session.config.is_posix {
                format!("cd {home}")
            } else {
                format!("Set-Location -LiteralPath {home}")
            };
            // Best-effort: if the steer fails the timed-out/dead handling
            // below evicts the session anyway.
            let _ = session.run(&cd_cmd, max_ms.min(5_000)).await;
            // Re-validate after the steer: the cwd is re-read from the live
            // shell, so a racing `cd` cannot slip an escape past the check.
            if !cwd_inside(&session.cwd().await, project_root) {
                self.sessions.lock().await.remove(&key);
                session.kill();
                return (
                    format!(
                        "shell left the workspace; reset to {}",
                        session_dir.to_string_lossy()
                    ),
                    None,
                    Some(session_dir.to_string_lossy().to_string()),
                    true,
                );
            }
        }
        let fallback_cwd = session.cwd().await;
        match session.run(command, max_ms).await {
            Ok((out, code, cwd)) => (out, code, Some(cwd), true),
            Err(err) => {
                // The shell may simply be gone — `exit`, or a killed parent. A
                // cached dead session would time out on every later call, so
                // evict it and let the next call spawn a fresh one.
                self.sessions.lock().await.remove(&key);
                // Eviction has to stop the process too: on a timeout the child
                // is owned by the reader thread, which only ever `wait`s on it,
                // so the command would otherwise run indefinitely.
                session.kill();
                (err, None, Some(fallback_cwd), true)
            }
        }
    }

    /// Fallback / direct-exec path: one child process, no state carried over.
    ///
    /// Shares the `kill_on_drop(true)` + `timeout` shape with the app-side
    /// `run_git` (`app/src/commands/workspace.rs`): both must kill the child
    /// on timeout/drop and bound the wait, or a hung subprocess outlives the
    /// call. Keep the two in step; see the cross-reference there.
    async fn run_once(
        &self,
        workdir: &std::path::Path,
        command: &str,
        cmd_args: &[String],
        max_ms: u64,
    ) -> (String, Option<i32>, Option<String>, bool) {
        let output = tokio::process::Command::new(command)
            .args(cmd_args)
            .current_dir(workdir)
            // Without this a timed-out build keeps running and holds the lock.
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(std::time::Duration::from_millis(max_ms), output).await {
            Ok(Ok(out)) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).to_string();
                if !out.stderr.is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                (
                    truncate_output(&combined),
                    out.status.code(),
                    Some(workdir.to_string_lossy().to_string()),
                    false,
                )
            }
            Ok(Err(err)) => (
                err.to_string(),
                None,
                Some(workdir.to_string_lossy().to_string()),
                false,
            ),
            Err(_) => (
                format!("timed out after {max_ms} ms"),
                None,
                Some(workdir.to_string_lossy().to_string()),
                false,
            ),
        }
    }
}

/// A short stable hash, used only to make the PTY sentinel unguessable.
fn stable_hash(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Single-quote a path for `cd`: the session directory comes from the
/// workspace root plus a validated `cwd`, but quoting still closes the
/// `'; rm -rf …` shape outright.
fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

fn powershell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "''"))
}

/// True when `cwd` (a live `$PWD` read back from the shell) resolves inside
/// `root`. Canonicalizes both sides so a symlinked cwd cannot fake
/// containment; an unresolvable cwd is treated as outside.
fn cwd_inside(cwd: &str, root: &std::path::Path) -> bool {
    let cwd_path = std::path::Path::new(cwd);
    if let Ok(canon) = canonicalize_portable(cwd_path) {
        return canon.starts_with(root);
    }
    // The cwd may have just been deleted from under the shell; fall back to
    // a lexical check rather than failing open or closed on a transient.
    cwd_path.starts_with(root)
}

/// Extract the cwd marker from the transcript tail.
///
/// The marker is emitted on its own line after the sentinel, but ConPTY may
/// wrap a long path (and PowerShell may prefix it with prompt/ANSI text). Join
/// the tail and strip escape sequences so a wrapped marker is still found.
fn cwd_from_transcript(text: &str, sentinel: &str, marker: &str) -> Option<String> {
    let sentinel_at = find_line(text, sentinel)?;
    let tail: String = text.lines().skip(sentinel_at + 1).collect();
    let tail = strip_terminal_escape_sequences(&tail);

    // The echoed wrapper script may also contain the marker text. Scan every
    // occurrence and accept the first one that parses as an absolute path.
    let mut rest = tail.as_str();
    while let Some(at) = rest.find(marker) {
        let candidate = &rest[at..];
        if let Some(path) = cwd_marker_path(candidate, marker) {
            return Some(path);
        }
        rest = &rest[at + marker.len()..];
    }
    None
}

/// Extract a live absolute cwd from a PTY line. The echoed `printf` command
/// also contains the marker text, so reject its non-path payload rather than
/// mistaking it for the result line.
fn cwd_marker_path(line: &str, marker: &str) -> Option<String> {
    let line = line.trim();
    let marker_at = line.find(marker)?;
    let rest = &line[marker_at + marker.len()..];
    let end = rest.rfind("__")?;
    let path = &rest[..end];
    std::path::Path::new(path)
        .is_absolute()
        .then(|| path.to_string())
}
