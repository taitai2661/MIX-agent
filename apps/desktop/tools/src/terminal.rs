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
pub fn detect_shell() -> ShellLaunchConfig {
    detect_shell_inner(std::env::var("MIX_SHELL").ok().as_deref())
}

fn detect_shell_inner(mix_shell_override: Option<&str>) -> ShellLaunchConfig {
    if let Some(custom) = mix_shell_override {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            let path = std::path::PathBuf::from(trimmed);
            let is_posix = !path
                .file_name()
                .map(|f| {
                    let s = f.to_string_lossy().to_ascii_lowercase();
                    s.contains("powershell") || s.contains("pwsh") || s.contains("cmd")
                })
                .unwrap_or(false);
            return ShellLaunchConfig {
                program: path,
                args: Vec::new(),
                is_posix,
            };
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(bash) = find_windows_bash() {
            return ShellLaunchConfig {
                program: bash,
                args: vec!["--login".into(), "-i".into()],
                is_posix: true,
            };
        }

        // Fallback to PowerShell on Windows when no bash is found
        ShellLaunchConfig {
            program: std::path::PathBuf::from("powershell.exe"),
            args: vec!["-NoLogo".into(), "-NoProfile".into()],
            is_posix: false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(sh) = std::env::var("SHELL") {
            let trimmed = sh.trim();
            if !trimmed.is_empty() {
                return ShellLaunchConfig {
                    program: std::path::PathBuf::from(trimmed),
                    args: Vec::new(),
                    is_posix: true,
                };
            }
        }

        for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
            let p = std::path::PathBuf::from(candidate);
            if p.is_file() {
                return ShellLaunchConfig {
                    program: p,
                    args: Vec::new(),
                    is_posix: true,
                };
            }
        }

        ShellLaunchConfig {
            program: std::path::PathBuf::from("/bin/sh"),
            args: Vec::new(),
            is_posix: true,
        }
    }
}

#[cfg(target_os = "windows")]
fn find_windows_bash() -> Option<std::path::PathBuf> {
    let fixed_candidates = [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        r"C:\tools\msys64\usr\bin\bash.exe",
        r"C:\msys64\usr\bin\bash.exe",
    ];
    for candidate in fixed_candidates {
        let p = std::path::PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let user_git = std::path::PathBuf::from(local_app_data).join(r"Programs\Git\bin\bash.exe");
        if user_git.is_file() {
            return Some(user_git);
        }
    }

    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let scoop_git =
            std::path::PathBuf::from(user_profile).join(r"scoop\apps\git\current\bin\bash.exe");
        if scoop_git.is_file() {
            return Some(scoop_git);
        }
    }

    find_executable_in_path("bash.exe")
}

#[cfg(target_os = "windows")]
fn find_executable_in_path(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
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
    /// Kill handle for the shell process. The reader thread owns the `Child`
    /// (it is the one calling `wait`), so the killer is cloned out of it and
    /// kept here: without it a timed-out command kept running for the rest of
    /// the session.
    killer: std::sync::Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    config: ShellLaunchConfig,
}

impl ShellSession {
    /// Spawn a login shell in `cwd`, or return `None` when no PTY is available.
    fn spawn(cwd: &str, id: &str) -> Option<Arc<Self>> {
        let config = detect_shell();
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: 40,
                cols: 160,
                pixel_width: 0,
                pixel_height: 0,
            })
            .ok()?;
        let mut cmd = portable_pty::CommandBuilder::new(&config.program);
        cmd.cwd(cwd);
        for arg in &config.args {
            cmd.arg(arg);
        }
        // A login-ish environment: without these, `cargo` and `nvm` are missing.
        cmd.env("TERM", "xterm-256color");
        cmd.env(
            "LANG",
            std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into()),
        );
        let mut child = pair.slave.spawn_command(cmd).ok()?;
        let killer = child.clone_killer();
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
                        // A blocking lock, never `try_lock`: the consumer holds
                        // this mutex while it scans up to 800 KB, and `try_lock`
                        // dropped every chunk that arrived in that window —
                        // silently missing output with no truncation marker.
                        let mut sink = sink.blocking_lock();
                        sink.extend_from_slice(&buf[..n]);
                        // Bound the buffer; the tail is what matters.
                        if sink.len() > MAX_OUTPUT_BYTES * 4 {
                            let drop_to = sink.len() - MAX_OUTPUT_BYTES * 4;
                            sink.drain(..drop_to);
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
            killer: std::sync::Mutex::new(killer),
            config,
        }))
    }

    /// Stop the shell process. Called when the session is evicted so a timed
    /// out command cannot keep running — or keep writing its exit code into a
    /// status file a replacement shell now owns.
    fn kill(&self) {
        let mut killer = match self.killer.lock() {
            Ok(k) => k,
            Err(_) => return,
        };
        if let Err(err) = killer.kill() {
            tracing::debug!(session = %self.id, %err, "shell kill failed");
        }
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
        // A command that names the protocol markers could forge its own
        // completion / cwd lines and smuggle a fake exit code. The markers
        // below are random per call, and any command reaching for the
        // protocol namespace is refused outright.
        // `anyhow`-style `{err:#}` keeps the full error chain (the `String`
        // boundary signature is kept so `ToolResult` callers are untouched).
        if command.contains("__MIXD_")
            || command.contains("__mixd_rc")
            || command.contains("mixd-exit")
        {
            return Err("command contains reserved shell-protocol markers".to_string());
        }
        let nonce = uuid::Uuid::new_v4().as_simple().to_string();
        let sentinel = format!("__MIXD_DONE_{}_{}__", self.id, nonce);
        let start_marker = format!("__MIXD_START_{}_{}__", self.id, nonce);
        let cwd_marker = format!("__MIXD_CWD_{}_{}__", self.id, nonce);
        // The status file is how the exit code comes back: a PTY has no other
        // channel for it.
        // Captured up front so the fallback below never has to block on the
        // mutex from inside the async context.
        let known_cwd = self.cwd.lock().await.clone();
        let status_path = std::env::temp_dir().join(format!("mixd-exit-{}-{nonce}", self.id));
        // Pre-create the status file owner-only: the shell's `>` keeps the
        // inode (and its mode) when it writes the exit code, so a predictable
        // /tmp path cannot be snooped by another local user.
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            if let Err(err) = opts.open(&status_path).map(|_| ()) {
                return Err(format!("status file setup failed: {err:#}"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&status_path, std::fs::Permissions::from_mode(0o600));
            }
        }
        // A start marker separates real command output from the PTY echo of
        // this script: the shell echoes every line it reads, so without it the
        // `__mixd_rc=$?` / `printf ...` lines leak into the result (the whole
        // script text was returned as the command's own output).
        let script = if self.config.is_posix {
            format!(
                "printf '\\n%s\\n' '{start}'\n{command}\n__mixd_rc=$?\nprintf '%s' \"$__mixd_rc\" > '{status}' 2>/dev/null\nprintf '\\n%s\\n' '{sentinel}'\nprintf '{cwd_marker}%s__\\n' \"$PWD\"\n",
                start = start_marker,
                status = status_path.display(),
                sentinel = sentinel,
                cwd_marker = cwd_marker,
            )
        } else {
            format!(
                "Write-Output \"`n{start}\"\n{command}\n$__mixd_rc = if ($LASTEXITCODE -ne $null) {{ $LASTEXITCODE }} elseif ($?) {{ 0 }} else {{ 1 }}\n[System.IO.File]::WriteAllText('{status}', \"$__mixd_rc\")\nWrite-Output \"`n{sentinel}\"\nWrite-Output \"{cwd_marker}$($ExecutionContext.SessionState.Path.CurrentLocation.Path)__\"\n",
                start = start_marker,
                status = status_path.display(),
                sentinel = sentinel,
                cwd_marker = cwd_marker,
            )
        };

        {
            let mut writer = self.writer.lock().await;
            writer
                .write_all(script.as_bytes())
                .map_err(|e| format!("shell write failed: {e:#}"))?;
            writer
                .flush()
                .map_err(|e| format!("shell flush failed: {e:#}"))?;
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

        // Real output lives between the start and done markers. The PTY echo
        // of the script (including `__mixd_rc=$?` and the `printf` lines) falls
        // outside that window, so it can no longer leak into the result.
        let mut out = slice_output(&text, &start_marker, &sentinel);
        while out.ends_with('\n') || out.ends_with('\r') {
            out.pop();
        }

        let exit_code = std::fs::read_to_string(&status_path).ok().and_then(|raw| {
            // Strict validation: a few ASCII digits (plus an optional
            // leading `-`), nothing else. A bloated or non-numeric file
            // means something other than our `printf` wrote it.
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.len() > 11 {
                return None;
            }
            let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            trimmed.parse::<i32>().ok()
        });
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

/// Slice real command output out of a raw PTY transcript: everything strictly
/// between the start and done markers. The shell echoes the whole injected
/// script (`__mixd_rc=$?`, the `printf` lines, …), which lives outside that
/// window and must never leak into the tool result.
fn slice_output(text: &str, start_marker: &str, sentinel: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let marker_at = find_line(text, sentinel).unwrap_or(lines.len());
    let begin = match find_line(text, start_marker) {
        // The marker itself is protocol, not output.
        Some(start_at) => (start_at + 1).min(marker_at).min(lines.len()),
        // No start marker (timeout residue, a killed shell): keep everything
        // up to the done marker rather than dropping the first line.
        None => 0,
    };
    let end = marker_at.min(lines.len());
    lines[begin..end].join("\n")
}

/// Cap the returned output, saying how much was dropped.
///
/// Shared with the workspace `git_diff` preview path via
/// [`truncate_to_char_boundary`]: both caps must cut on a character boundary
/// (the release profile is `panic = "abort"`), so the boundary walk lives in
/// one place. See `app/src/commands/workspace.rs` (`git_diff` truncation).
fn truncate_output(text: &str) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text.to_string();
    }
    let head = truncate_to_char_boundary(text, MAX_OUTPUT_BYTES);
    let cut = head.len();
    let kept_lines = head.lines().count();
    format!(
        "{head}\n… [truncated: {cut} of {} bytes shown, {kept_lines} lines]",
        text.len()
    )
}

/// Cut `text` to at most `max_bytes` on a character boundary.
///
/// `pub` so the app-side `git_diff` truncation reuses the exact same walk
/// instead of duplicating it (a byte cut inside a multi-byte character
/// panics, and the release profile aborts the whole app).
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
        let workdir = match std::fs::canonicalize(&workdir) {
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
                let canon = match std::fs::canonicalize(&candidate) {
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
            self.run_in_shell(&workdir, &session_dir, &command, max_ms)
                .await
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
        command: &str,
        max_ms: u64,
    ) -> (String, Option<i32>, Option<String>, bool) {
        let key = session_dir.to_string_lossy().to_string();
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
                    let spawned = ShellSession::spawn(&key, &session_id);
                    if let Some(session) = &spawned {
                        sessions.insert(key.clone(), session.clone());
                    }
                    spawned
                }
            }
        };
        let Some(session) = session else {
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
                key
            })), false);
        };
        // A previous `cd` outside the project must not persist: steer the
        // shell back before running so turn N+1 does not execute in `/etc`
        // because turn N ended there. Both sides are canonicalized so a
        // symlinked `$PWD` cannot fake containment.
        if !cwd_inside(&session.cwd().await, project_root) {
            let home = shell_quote(&key);
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

/// True when `cwd` (a live `$PWD` read back from the shell) resolves inside
/// `root`. Canonicalizes both sides so a symlinked cwd cannot fake
/// containment; an unresolvable cwd is treated as outside.
fn cwd_inside(cwd: &str, root: &std::path::Path) -> bool {
    let cwd_path = std::path::Path::new(cwd);
    if let Ok(canon) = std::fs::canonicalize(cwd_path) {
        return canon.starts_with(root);
    }
    // The cwd may have just been deleted from under the shell; fall back to
    // a lexical check rather than failing open or closed on a transient.
    cwd_path.starts_with(root)
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
            additional_paths: Vec::new(),
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
    fn truncating_multibyte_output_does_not_panic() {
        // Byte `MAX_OUTPUT_BYTES` is not a character boundary when the output is
        // Japanese, and slicing there panicked — the release profile aborts.
        let big = "あ".repeat(MAX_OUTPUT_BYTES);
        let out = truncate_output(&big);
        assert!(out.contains("[truncated:"));
        assert!(out.len() < big.len());
    }

    #[test]
    fn short_output_is_untouched() {
        assert_eq!(truncate_output("hello\n"), "hello\n");
    }

    #[test]
    fn the_char_boundary_walk_is_shared_with_the_workspace_preview() {
        // `truncate_to_char_boundary` is the single boundary walk behind both
        // `truncate_output` here and the `git_diff` truncation in
        // `app/src/commands/workspace.rs`. Pin the contract: never longer
        // than the cap, always a boundary, empty only for a zero cap.
        assert_eq!(truncate_to_char_boundary("hello", 5), "hello");
        assert_eq!(truncate_to_char_boundary("hello", 3), "hel");
        assert_eq!(truncate_to_char_boundary("あいう", 4), "あ");
        assert_eq!(truncate_to_char_boundary("あいう", 0), "");
    }

    #[test]
    fn the_sentinel_is_derived_from_the_workspace() {
        // Two workspaces must not share a sentinel, or a command in one can
        // consume the completion marker of the other.
        assert_ne!(stable_hash("/a/project"), stable_hash("/b/project"));
        assert_eq!(stable_hash("/a/project"), stable_hash("/a/project"));
    }

    #[test]
    fn pty_echo_of_the_wrapper_script_is_not_returned_as_output() {
        // Regression test for the transcript showing the injected script
        // (`__mixd_rc=$?`, `printf ... MIXD_DONE ...`, prompt escapes) as the
        // command's own stdout. Only lines between the markers are output.
        let start = "__MIXD_START_1__";
        let done = "__MIXD_DONE_1__";
        let transcript = [
            "user@host app % printf '\\n%s\\n' '__MIXD_START_1__';",
            "date -u; date",
            "             __mixd_rc=$?",
            "             printf '%s' \"$__mixd_rc\" > '/tmp/mixd-exit-foo' 2>/dev/null",
            "             printf '\\n%s\\n' '__MIXD_DONE_1__'",
            start,
            "2026-10-12",
            done,
            "__MIXD_CWD_1__/tmp__",
        ]
        .join("\n");
        assert_eq!(slice_output(&transcript, start, done), "2026-10-12");
    }

    #[test]
    fn missing_markers_keep_the_transcript_instead_of_dropping_a_line() {
        // Timeout residue or a killed shell may have no markers at all: the
        // old `unwrap_or(0) + 1` dropped the first line in that case.
        let done = "__MIXD_DONE_1__";
        assert_eq!(
            slice_output("one\ntwo", "__MIXD_START_1__", done),
            "one\ntwo"
        );
        assert_eq!(
            slice_output("one\ntwo", "__MIXD_START_1__", "__MIXD_DONE_X__"),
            "one\ntwo"
        );
    }

    #[test]
    fn shell_quote_neutralises_command_breakout() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(
            shell_quote("a'; rm -rf /; echo '"),
            "'a'\\''; rm -rf /; echo '\\'''"
        );
    }

    #[tokio::test]
    async fn the_shell_state_persists_between_calls() {
        let tool = TerminalTool::default();
        let (context, dir) = ctx_for("persist");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        let first = tool
            .invoke(context.clone(), serde_json::json!({ "command": "cd sub" }))
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
                out.contains("sub"),
                "`cd` must survive into the next call, got {out:?}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_directory_escape_does_not_persist_into_the_next_call() {
        let tool = TerminalTool::default();
        let (context, dir) = ctx_for("escape");

        let first = tool
            .invoke(context.clone(), serde_json::json!({ "command": "cd /tmp" }))
            .await
            .expect("first call");
        if first.content["persistent"] != true {
            std::fs::remove_dir_all(&dir).ok();
            return; // no PTY on this platform
        }
        let second = tool
            .invoke(context.clone(), serde_json::json!({ "command": "pwd" }))
            .await
            .expect("second call");
        let out = second.content["stdout"].as_str().unwrap_or_default();
        let wanted = dir.to_string_lossy().to_string();
        assert!(
            out.contains(&wanted) || out.contains("escape"),
            "the shell must be steered back into the workspace, got {out:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_explicit_cwd_outside_the_workspace_is_rejected() {
        let tool = TerminalTool::default();
        let (context, dir) = ctx_for("cwd-reject");
        let result = tool
            .invoke(
                context,
                serde_json::json!({ "command": "pwd", "cwd": "/etc" }),
            )
            .await
            .expect("invoke");
        assert!(!result.ok);
        assert!(result
            .error
            .unwrap_or_default()
            .contains("outside the workspace"));
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

    #[test]
    fn detect_shell_returns_valid_config() {
        let config = detect_shell();
        assert!(!config.program.as_os_str().is_empty());
        #[cfg(not(target_os = "windows"))]
        {
            assert!(config.is_posix);
        }
    }

    #[test]
    fn mix_shell_override_works() {
        let config = detect_shell_inner(Some("/custom/bin/bash"));
        assert_eq!(config.program, std::path::PathBuf::from("/custom/bin/bash"));
        assert!(config.is_posix);

        let config_zsh = detect_shell_inner(Some("/usr/local/bin/zsh"));
        assert_eq!(
            config_zsh.program,
            std::path::PathBuf::from("/usr/local/bin/zsh")
        );
        assert!(config_zsh.is_posix);

        let config_pwsh = detect_shell_inner(Some("C:\\Windows\\pwsh.exe"));
        assert_eq!(
            config_pwsh.program,
            std::path::PathBuf::from("C:\\Windows\\pwsh.exe")
        );
        assert!(!config_pwsh.is_posix);

        let config_cmd = detect_shell_inner(Some("C:\\Windows\\System32\\cmd.exe"));
        assert_eq!(
            config_cmd.program,
            std::path::PathBuf::from("C:\\Windows\\System32\\cmd.exe")
        );
        assert!(!config_cmd.is_posix);
    }
}
