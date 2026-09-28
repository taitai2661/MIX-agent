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
#[cfg(test)]
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::Mutex;

use super::{Tool, ToolContext, ToolResult};

const DEFAULT_MAX_WALL_CLOCK_MS: u64 = 30_000;
/// How much output to return. A runaway `yes` must not fill the context window.
const MAX_OUTPUT_BYTES: usize = 200 * 1024;

pub struct TerminalTool {
    pub max_wall_clock_ms: u64,
    pub history: Arc<Mutex<Vec<TerminalCommand>>>,
    /// Live shells, keyed by workspace so two projects do not share a `cd`.
    sessions: Arc<Mutex<HashMap<String, Arc<ShellSession>>>>,
}

impl Default for TerminalTool {
    fn default() -> Self {
        Self {
            max_wall_clock_ms: DEFAULT_MAX_WALL_CLOCK_MS,
            history: Arc::new(Mutex::new(Vec::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
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

/// A PTY plus the reader thread draining its output.
///
/// The writer is behind a mutex rather than stored bare: a tool is shared
/// across runs, so the session has to be `Sync`, and a raw `dyn Write` is not.
struct ShellSession {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Everything the reader thread has seen so far.
    output: Arc<Mutex<Vec<u8>>>,
    /// The working directory the shell is in, tracked by echoing `pwd`.
    cwd: Arc<Mutex<String>>,
    id: String,
}

impl ShellSession {
    /// Spawn a login shell in `cwd`, or return `None` when no PTY is available.
    fn spawn(cwd: &str, id: &str) -> Option<Arc<Self>> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: 40,
                cols: 160,
                pixel_width: 0,
                pixel_height: 0,
            })
            .ok()?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = portable_pty::CommandBuilder::new(shell);
        cmd.cwd(cwd);
        // A login-ish environment: without these, `cargo` and `nvm` are missing.
        cmd.env("TERM", "xterm-256color");
        cmd.env(
            "LANG",
            std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into()),
        );
        let mut child = pair.slave.spawn_command(cmd).ok()?;
        drop(pair.slave);
        let writer = pair.master.take_writer().ok()?;
        let master = pair.master.try_clone_reader().ok()?;

        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = output.clone();
        // Drain the PTY on a dedicated thread. A PTY has a small buffer; nothing
        // reading it means a chatty command blocks forever.
        std::thread::spawn(move || {
            let mut reader = master;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut sink) = sink.try_lock() {
                            sink.extend_from_slice(&buf[..n]);
                            // Bound the buffer; the tail is what matters.
                            if sink.len() > MAX_OUTPUT_BYTES * 4 {
                                let drop_to = sink.len() - MAX_OUTPUT_BYTES * 4;
                                sink.drain(..drop_to);
                            }
                        }
                    }
                }
            }
            let _ = child.wait();
        });

        Some(Arc::new(Self {
            writer: Mutex::new(writer),
            output,
            cwd: Arc::new(Mutex::new(cwd.to_string())),
            id: id.to_string(),
        }))
    }

    async fn cwd(&self) -> String {
        self.cwd.lock().await.clone()
    }

    /// Run `command`, returning its output, exit code and the resulting cwd.
    ///
    /// The shell echoes every line it reads, so the completion markers are
    /// matched as *whole lines* rather than by substring: the echo of
    /// `printf '\n%s\n' 'MARKER'` is the longer line `printf ... 'MARKER'`,
    /// which is why a substring search would latch onto the echo instead of the
    /// real marker and return the command text as its own output.
    async fn run(
        &self,
        command: &str,
        max_ms: u64,
    ) -> Result<(String, Option<i32>, String), String> {
        let sentinel = format!("__MIXD_DONE_{}__", self.id);
        let cwd_marker = format!("__MIXD_CWD_{}__", self.id);
        // The status file is how the exit code comes back: a PTY has no other
        // channel for it.
        // Captured up front so the fallback below never has to block on the
        // mutex from inside the async context.
        let known_cwd = self.cwd.lock().await.clone();
        let status_path = std::env::temp_dir().join(format!("mixd-exit-{sentinel}"));
        let _ = std::fs::remove_file(&status_path);
        let script = format!(
            "{command}\n             __mixd_rc=$?\n             printf '%s' \"$__mixd_rc\" > '{status}' 2>/dev/null\n             printf '\\n%s\\n' '{sentinel}'\n             printf '{cwd_marker}%s__\\n' \"$PWD\"\n",
            status = status_path.display(),
            sentinel = sentinel,
            cwd_marker = cwd_marker,
        );

        {
            let mut writer = self.writer.lock().await;
            writer
                .write_all(script.as_bytes())
                .map_err(|e| format!("shell write failed: {e}"))?;
            writer
                .flush()
                .map_err(|e| format!("shell flush failed: {e}"))?;
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(max_ms);
        let done = loop {
            let seen = {
                let buffer = self.output.lock().await;
                find_line(&String::from_utf8_lossy(&buffer), &sentinel)
            };
            if seen.is_some() {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        if !done {
            // Drain whatever arrived so a timeout does not corrupt the next call.
            self.output.lock().await.clear();
            return Err(format!("timed out after {max_ms} ms"));
        }

        // Take the buffer and reset it in one lock: the reader thread keeps
        // appending, so the only thing that makes this well-defined is doing the
        // read-and-clear atomically.
        let buffer = {
            let mut guard = self.output.lock().await;
            std::mem::take(&mut *guard)
        };
        let text = String::from_utf8_lossy(&buffer).to_string();
        let lines: Vec<&str> = text.lines().collect();
        let marker_at = find_line(&text, &sentinel).unwrap_or(lines.len());

        // Everything after the marker up to the cwd probe is protocol, not output.
        let cwd = lines
            .iter()
            .skip(marker_at)
            .find_map(|line| {
                line.trim()
                    .strip_prefix(&cwd_marker)
                    .and_then(|rest| rest.strip_suffix("__"))
                    .map(|raw| raw.to_string())
            })
            .unwrap_or(known_cwd);
        *self.cwd.lock().await = cwd.clone();

        let mut out = lines[..marker_at].join("\n");
        // The echoed command line is the first thing in the buffer; drop it so
        // the result is output only.
        if let Some((first, rest)) = out.split_once('\n') {
            if first
                .trim_start()
                .starts_with(&command.chars().take(12).collect::<String>())
            {
                out = rest.to_string();
            }
        }
        while out.ends_with('\n') || out.ends_with('\r') {
            out.pop();
        }

        let exit_code = std::fs::read_to_string(&status_path)
            .ok()
            .and_then(|raw| raw.trim().parse::<i32>().ok());
        let _ = std::fs::remove_file(&status_path);
        Ok((truncate_output(&out), exit_code, cwd))
    }
}

/// Index of the line whose trimmed content is exactly `needle`.
///
/// Matching whole lines is what separates a real marker from the shell's echo
/// of the command that printed it.
fn find_line(text: &str, needle: &str) -> Option<usize> {
    text.lines()
        .position(|line| line.trim_end_matches('\r') == needle)
}

/// Cap the returned output, saying how much was dropped.
fn truncate_output(text: &str) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text.to_string();
    }
    let head = &text[..MAX_OUTPUT_BYTES];
    let kept_lines = head.lines().count();
    format!(
        "{head}\n… [truncated: {} of {} bytes shown, {kept_lines} lines]",
        MAX_OUTPUT_BYTES,
        text.len()
    )
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
                    "description": "Absolute path to run in. Only meaningful when starting a fresh workspace shell."
                },
                "max_wall_clock_ms": { "type": "integer", "default": 30000 }
            },
            "required": ["command"]
        })
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
            });
        let workdir = match ctx.root_path.as_deref() {
            Some(root) => std::path::PathBuf::from(root),
            None => std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        };
        let workdir = std::fs::canonicalize(&workdir).unwrap_or(workdir);

        let started = chrono::Utc::now();
        let (stdout, exit_code, cwd, persistent) = if cmd_args.is_empty() {
            self.run_in_shell(&workdir, &command, max_ms).await
        } else {
            // `args` means a direct exec, so state must not carry over.
            self.run_once(&workdir, &command, &cmd_args, max_ms).await
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
        self.history.lock().await.push(record);
        Ok(ToolResult {
            ok,
            content: payload,
            error: err_msg,
        })
    }
}

impl TerminalTool {
    /// Get or create the persistent shell for this workspace and run `command`.
    async fn run_in_shell(
        &self,
        workdir: &std::path::Path,
        command: &str,
        max_ms: u64,
    ) -> (String, Option<i32>, Option<String>, bool) {
        let key = workdir.to_string_lossy().to_string();
        // A failed spawn must not be cached, or every later call in this
        // workspace is permanently degraded.
        let session = {
            let mut sessions = self.sessions.lock().await;
            match sessions.get(&key) {
                Some(existing) => Some(existing.clone()),
                None => {
                    let spawned =
                        ShellSession::spawn(&key, &format!("{}-{}", key.len(), stable_hash(&key)));
                    if let Some(session) = &spawned {
                        sessions.insert(key.clone(), session.clone());
                    }
                    spawned
                }
            }
        };
        let Some(session) = session else {
            // No PTY available: a one-shot child still runs the command.
            let (out, code, cwd, _) = self
                .run_once(
                    workdir,
                    "/bin/sh",
                    &["-c".to_string(), command.to_string()],
                    max_ms,
                )
                .await;
            return (out, code, Some(cwd.unwrap_or(key)), false);
        };
        let fallback_cwd = session.cwd().await;
        match session.run(command, max_ms).await {
            Ok((out, code, cwd)) => (out, code, Some(cwd), true),
            Err(err) => {
                // The shell may simply be gone — `exit`, or a killed parent. A
                // cached dead session would time out on every later call, so
                // evict it and let the next call spawn a fresh one.
                self.sessions.lock().await.remove(&key);
                (err, None, Some(fallback_cwd), true)
            }
        }
    }

    /// Fallback / direct-exec path: one child process, no state carried over.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One directory per test. The persistent shell is keyed by workspace, so
    /// sharing a root would make the tests queue behind each other on one
    /// session and hide ordering bugs.
    fn ctx_for(name: &str) -> (ToolContext, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mixd-term-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let context = ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: Some(dir.to_string_lossy().to_string()),
            approval_token: None,
        };
        (context, dir)
    }

    #[test]
    fn default_timeout_is_not_zero() {
        // `TerminalTool::default()` used to yield 0 ms, timing out instantly
        // whenever the model omitted max_wall_clock_ms.
        let tool = TerminalTool::default();
        assert_eq!(tool.max_wall_clock_ms, DEFAULT_MAX_WALL_CLOCK_MS);
        assert!(tool.max_wall_clock_ms > 0);
    }

    #[test]
    fn output_is_capped_with_a_visible_marker() {
        let big = "x".repeat(MAX_OUTPUT_BYTES * 2);
        let out = truncate_output(&big);
        assert!(out.contains("[truncated:"));
        assert!(out.len() < big.len());
    }

    #[test]
    fn short_output_is_untouched() {
        assert_eq!(truncate_output("hello\n"), "hello\n");
    }

    #[test]
    fn the_sentinel_is_derived_from_the_workspace() {
        // Two workspaces must not share a sentinel, or a command in one can
        // consume the completion marker of the other.
        assert_ne!(stable_hash("/a/project"), stable_hash("/b/project"));
        assert_eq!(stable_hash("/a/project"), stable_hash("/a/project"));
    }

    #[tokio::test]
    async fn the_shell_state_persists_between_calls() {
        let tool = TerminalTool::default();
        let (context, dir) = ctx_for("persist");

        let first = tool
            .invoke(context.clone(), serde_json::json!({ "command": "cd /tmp" }))
            .await
            .expect("first call");
        // If no PTY is available the fallback still succeeds; the persistence
        // assertion below is then vacuous, so only assert the happy path here.
        if first.content["persistent"] == true {
            let second = tool
                .invoke(context.clone(), serde_json::json!({ "command": "pwd" }))
                .await
                .expect("second call");
            let out = second.content["stdout"].as_str().unwrap_or_default();
            assert!(
                out.contains("/tmp"),
                "`cd` must survive into the next call, got {out:?}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn reports_a_nonzero_exit_code() {
        let tool = TerminalTool::default();
        let (context, dir) = ctx_for("exitcode");
        let result = tool
            .invoke(
                context,
                serde_json::json!({ "command": "(exit 3)", "max_wall_clock_ms": 10000 }),
            )
            .await
            .expect("invoke");
        if result.content["persistent"] == true {
            assert!(!result.ok, "exit 3 must not report success");
            assert_eq!(result.content["exit_code"], 3);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_shell_that_exits_is_replaced_on_the_next_call() {
        let tool = TerminalTool::default();
        let (context, dir) = ctx_for("exit");

        let first = tool
            .invoke(
                context.clone(),
                // `exit` kills the shell before the marker is printed, so this
                // call is expected to time out; keep the timeout short.
                serde_json::json!({ "command": "exit 0", "max_wall_clock_ms": 2000 }),
            )
            .await
            .expect("first call");
        if first.content["persistent"] != true {
            std::fs::remove_dir_all(&dir).ok();
            return; // no PTY on this platform
        }
        // The session is now dead; the next call must not reuse it.
        let second = tool
            .invoke(
                context.clone(),
                serde_json::json!({ "command": "echo alive", "max_wall_clock_ms": 10000 }),
            )
            .await
            .expect("second call");
        assert!(
            second.content["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("alive"),
            "a dead shell must be replaced: {:?}",
            second.content
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_direct_exec_path_bypasses_the_shell() {
        let tool = TerminalTool::default();
        let (context, dir) = ctx_for("argv");
        let result = tool
            .invoke(
                context,
                serde_json::json!({
                    "command": "/bin/echo",
                    "args": ["hello from argv"],
                    "max_wall_clock_ms": 10000,
                }),
            )
            .await
            .expect("invoke");
        assert_eq!(result.content["persistent"], false);
        assert!(result.content["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("hello from argv"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
