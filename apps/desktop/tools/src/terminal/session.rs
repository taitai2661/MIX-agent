use super::*;

/// The per-command wrapper emitted into the PTY.
pub(crate) struct ShellProtocol {
    pub(crate) script: String,
    pub(crate) sentinel: String,
    pub(crate) start_marker: String,
    pub(crate) cwd_marker: String,
    pub(crate) status_path: std::path::PathBuf,
}

impl ShellSession {
    /// Spawn a login shell in `cwd`, or return `None` when no PTY is available.
    pub(crate) fn spawn(
        cwd: &str,
        id: &str,
        sandbox_root: Option<&std::path::Path>,
        sandbox_network: bool,
        sandbox_strict: bool,
    ) -> Option<Arc<Self>> {
        let config = detect_shell();
        let config = match sandbox_root {
            Some(root) => {
                match sandboxed_shell_config(&config, root, sandbox_network, sandbox_strict) {
                    Some(sandboxed) => sandboxed,
                    None => {
                        // Windows attaches a Job Object after spawn instead of
                        // wrapping the command, so this is expected there.
                        #[cfg(not(windows))]
                        {
                            // Fail closed: a requested sandbox that cannot be built
                            // must refuse to spawn rather than degrade to an
                            // unsandboxed shell. The caller (`run_in_shell`) turns
                            // this `None` into an explicit error, and the settings
                            // pane refuses to persist an unenforceable opt-in.
                            tracing::warn!(
                                "terminal sandbox is requested but unavailable; \
                             refusing to spawn an unsandboxed shell"
                            );
                            return None;
                        }
                        #[cfg(windows)]
                        config
                    }
                }
            }
            None => config,
        };
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
        let shell_cwd = normalize_windows_path(std::path::Path::new(cwd));
        cmd.cwd(&shell_cwd);
        for arg in &config.args {
            cmd.arg(arg);
        }
        // A login-ish environment: without these, `cargo` and `nvm` are missing.
        cmd.env("TERM", "xterm-256color");
        cmd.env(
            "LANG",
            std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into()),
        );
        // Choose where the hook/status files live *before* spawning: a Linux
        // sandbox hides the host /tmp from the shell, so a status file in the
        // temp dir can never be read back by the parent. Both helpers bind the
        // workspace root writable, so a private directory underneath it works.
        let status_dir = match (cfg!(target_os = "linux"), sandbox_root) {
            (true, Some(root)) => {
                let dir = root.join(".mixd-status");
                if let Err(err) = std::fs::create_dir_all(&dir) {
                    tracing::warn!(?err, path = %dir.display(), "sandboxed terminal: cannot create status directory; refusing to spawn");
                    return None;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                        .is_err()
                    {
                        tracing::warn!(path = %dir.display(), "sandboxed terminal: cannot protect status directory");
                        return None;
                    }
                }
                dir
            }
            _ => std::env::temp_dir(),
        };
        let mut child = pair.slave.spawn_command(cmd).ok()?;
        // Windows has no `sandbox-exec`-style pre-exec hook reachable through
        // portable-pty, so the job is attached to the live process. Every
        // descendant joins it; closing the job kills the tree. Fail closed:
        // when the sandbox was requested but the Job Object cannot be
        // attached, the freshly spawned shell is killed and `None` is
        // returned so the caller reports an error instead of running
        // unconfined.
        #[cfg(windows)]
        let sandbox_job = if sandbox_root.is_some() {
            match child
                .as_raw_handle()
                .and_then(|handle| attach_job_object(handle as _, 64))
            {
                Some(job) => Some(job),
                None => {
                    tracing::warn!(
                        "terminal sandbox is requested but the Job Object could not be attached; \
                         refusing to run unsandboxed"
                    );
                    let mut killer = child.clone_killer();
                    let _ = killer.kill();
                    return None;
                }
            }
        } else {
            None
        };
        let killer = child.clone_killer();
        drop(pair.slave);
        let writer = pair.master.take_writer().ok()?;
        let master = pair.master.try_clone_reader().ok()?;
        // The prompt hook appends one line here after every command. The file
        // is created owner-only because its path is predictable.
        let done_path = status_dir.join(format!(
            "mixd-done-{}-{}",
            id,
            uuid::Uuid::new_v4().as_simple()
        ));
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            opts.open(&done_path).ok()?;
        }
        // `pair.master` is dropped at the end of this function, so keep a
        // duplicate of the control fd for `tcgetpgrp`/`kill` later.
        #[cfg(unix)]
        let interrupt_fd = {
            use std::os::fd::{FromRawFd, OwnedFd};
            pair.master.as_raw_fd().and_then(|fd| {
                let duplicated = unsafe { libc::dup(fd) };
                if duplicated < 0 {
                    None
                } else {
                    Some(unsafe { OwnedFd::from_raw_fd(duplicated) })
                }
            })
        };

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
            #[cfg(windows)]
            sandbox_job: std::sync::Mutex::new(sandbox_job),
            config,
            busy: std::sync::atomic::AtomicBool::new(false),
            warmed: std::sync::atomic::AtomicBool::new(false),
            #[cfg(unix)]
            interrupt_fd: std::sync::Mutex::new(interrupt_fd),
            status_dir,
            done_path,
            hook_ready: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// Stop the shell process. Called when the session is evicted so a timed
    /// out command cannot keep running — or keep writing its exit code into a
    /// status file a replacement shell now owns.
    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn kill(&self) {
        let _ = std::fs::remove_file(&self.done_path);
        // Remove the sandbox-specific status directory when it is empty. Never
        // touch the shared temp dir (it may be empty on a fresh machine).
        if self
            .status_dir
            .file_name()
            .is_some_and(|name| name == ".mixd-status")
        {
            let _ = std::fs::remove_dir(&self.status_dir);
        }
        let mut killer = match self.killer.lock() {
            Ok(k) => k,
            Err(_) => return,
        };
        if let Err(err) = killer.kill() {
            tracing::debug!(session = %self.id, %err, "shell kill failed");
        }
        #[cfg(windows)]
        if let Ok(mut job) = self.sandbox_job.lock() {
            if let Some(job) = job.as_mut() {
                job.close();
            }
        }
    }

    pub(crate) async fn cwd(&self) -> String {
        self.cwd.lock().await.clone()
    }

    /// Wait (bounded) for the shell's first output before writing to it.
    ///
    /// Interactive Git Bash sources a login profile before it starts reading
    /// input. On a loaded Windows runner that can take long enough that the
    /// first command is consumed by the startup and never runs, which surfaces
    /// as a protocol-marker timeout. The wait is best-effort: a shell that
    /// stays silent is still used after `max_ms`, so short-lived test shells
    /// and non-interactive shells are not blocked indefinitely.
    pub(crate) async fn ensure_warmed(&self, max_ms: u64) {
        if self.warmed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(max_ms);
        loop {
            if !self.output.lock().await.is_empty() {
                return;
            }
            if std::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// File the shell's prompt hook appends to after each command.
    pub(crate) fn done_path(&self) -> &std::path::Path {
        &self.done_path
    }

    /// Install a prompt hook that appends one line to [`Self::done_path`].
    ///
    /// The streaming terminal cannot pre-write a completion script: an
    /// interactive command (`cat`, a REPL) reads stdin and would consume it.
    /// Instead the shell itself writes a marker after the command exits, and
    /// the status query is typed only then. This works even when a theme owns
    /// `PS1`, which is why it does not try to parse the prompt text.
    pub(crate) async fn ensure_done_hook(&self, timeout_ms: u64) -> Result<(), String> {
        if self.hook_ready.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let path = native_path_string(&self.done_path);
        let setup = if self.config.is_posix {
            let quoted = shell_quote(&path);
            format!(
                "__mixd_done_hook() {{ printf 'done\\n' >> {quoted}; }}\n\
                 if [ -n \"$ZSH_VERSION\" ]; then\n\
                   precmd_functions+=(__mixd_done_hook)\n\
                 else\n\
                   PROMPT_COMMAND=\"__mixd_done_hook${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\"\n\
                 fi\n"
            )
        } else {
            let quoted = path.replace('\'', "''");
            format!(
                "function prompt {{ Add-Content -LiteralPath '{quoted}' -Value 'done'; 'PS> ' }}\r\n"
            )
        };
        self.write_raw(setup.as_bytes())
            .await
            .map_err(|err| format!("could not install the completion hook: {err}"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if std::fs::metadata(&self.done_path)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)
            {
                self.hook_ready
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "the shell did not run the terminal prompt hook; interactive mode is unavailable"
                        .to_string(),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// Mark the shell as owned by a command. The streaming terminal does not
    /// go through [`Self::run`]'s guard, so it has to set this itself or shell
    /// eviction could kill the live session between two polls.
    pub(crate) fn set_busy(&self, busy: bool) {
        self.busy.store(busy, std::sync::atomic::Ordering::SeqCst);
    }

    /// Current length of the raw PTY buffer. Used as the base offset for a
    /// streamed command so earlier residue is not replayed.
    pub(crate) async fn buffer_len(&self) -> usize {
        self.output.lock().await.len()
    }

    /// Everything the shell has written since `base`. The second value is
    /// `true` when the buffer was trimmed past `base`, in which case the caller
    /// must treat the output as a fresh snapshot rather than an append.
    pub(crate) async fn snapshot_from(&self, base: usize) -> (String, bool) {
        let buffer = self.output.lock().await;
        let start = base.min(buffer.len());
        (
            String::from_utf8_lossy(&buffer[start..]).to_string(),
            start != base,
        )
    }

    /// Write bytes to the PTY without waiting for a command protocol.
    pub(crate) async fn write_raw(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut writer = self.writer.lock().await;
        writer.write_all(bytes)?;
        writer.flush()
    }

    /// Deliver an interrupt to the terminal's foreground process group.
    ///
    /// A raw-mode slave (portable-pty's default) does not turn `\x03` into
    /// SIGINT, so the signal is sent directly on Unix. On Windows there is no
    /// portable equivalent here, so the byte is the fallback. Writing the byte
    /// *after* a successful Unix signal is actively harmful: once the child has
    /// died the byte can interrupt the shell's own completion wrapper, and the
    /// command then never reports its exit status.
    pub(crate) async fn interrupt(&self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let sent = match self.interrupt_fd.lock() {
                Ok(guard) => guard.as_ref().is_some_and(|fd| {
                    let pgrp = unsafe { libc::tcgetpgrp(fd.as_raw_fd()) };
                    pgrp > 0 && unsafe { libc::kill(-pgrp, libc::SIGINT) } == 0
                }),
                Err(_) => false,
            };
            if sent {
                return;
            }
        }
        let _ = self.write_raw(b"\x03").await;
    }

    /// Drop the buffered transcript. Called when a streamed command finishes so
    /// the next one does not replay megabytes of history.
    pub(crate) async fn clear_output(&self) {
        self.output.lock().await.clear();
    }

    /// Build the marker/status-file wrapper for one command.
    ///
    /// Shared by the blocking [`Self::run`] and the streaming terminal path so
    /// both use exactly the same completion protocol.
    pub(crate) fn build_protocol(&self, command: &str) -> Result<ShellProtocol, String> {
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
        // Use the same directory as the prompt-hook file. On a sandboxed Linux
        // shell this is inside the workspace (the sandbox's private /tmp is
        // invisible to us); elsewhere it is the process temp dir.
        let status_path = self
            .status_dir
            .join(format!("mixd-exit-{}-{nonce}", self.id));
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
        // this script. Run the POSIX command with eval so its state changes
        // persist, and capture `$?` on the same input line: interactive Git
        // Bash prompt hooks run between input lines and can otherwise replace
        // the command's exit status before the wrapper reads it.
        let status_str = native_path_string(&status_path);
        let status_expr = posix_path_expression(&status_path);
        let cwd_expr = posix_cwd_expression();
        let script = if self.config.is_posix {
            format!(
                "printf '\\n%s\\n' '{start}'\neval {command}; __mixd_rc=$?\nprintf '%s' \"$__mixd_rc\" > {status} 2>/dev/null\nprintf '\\n%s\\n' '{sentinel}'\nprintf '{cwd_marker}%s__\\n' \"{cwd}\"\n",
                start = start_marker,
                command = shell_quote(command),
                status = status_expr,
                sentinel = sentinel,
                cwd_marker = cwd_marker,
                cwd = cwd_expr,
            )
        } else {
            // PowerShell in raw PTY mode handles CRLF reliably for interactive
            // evaluation.
            format!(
                "Write-Output \"`n{start}\"\r\n{command}\r\n$__mixd_rc = if ($LASTEXITCODE -ne $null) {{ $LASTEXITCODE }} elseif ($?) {{ 0 }} else {{ 1 }}\r\nif ($__mixd_rc -eq $null) {{ $__mixd_rc = 0 }}\r\n[System.IO.File]::WriteAllText('{status}', \"$__mixd_rc\", [System.Text.Encoding]::ASCII)\r\nWrite-Output \"`n{sentinel}\"\r\nWrite-Output \"{cwd_marker}$($ExecutionContext.SessionState.Path.CurrentLocation.Path)__\"\r\n",
                start = start_marker,
                status = status_str.replace('\'', "''"),
                sentinel = sentinel,
                cwd_marker = cwd_marker,
            )
        };
        Ok(ShellProtocol {
            script,
            sentinel,
            start_marker,
            cwd_marker,
            status_path,
        })
    }

    /// Run `command`, returning its output, exit code and the resulting cwd.
    ///
    /// The shell echoes every line it reads, so the completion markers are
    /// matched as *whole lines* rather than by substring: the echo of
    /// `printf '\n%s\n' 'MARKER'` is the longer line `printf ... 'MARKER'`,
    /// which is why a substring search would latch onto the echo instead of the
    /// real marker and return the command text as its own output.
    pub(crate) async fn run(
        &self,
        command: &str,
        max_ms: u64,
    ) -> Result<(String, Option<i32>, String), String> {
        struct BusyGuard<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for BusyGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        self.busy.store(true, std::sync::atomic::Ordering::SeqCst);
        let _busy = BusyGuard(&self.busy);
        // A command that names the protocol markers could forge its own
        // completion / cwd lines and smuggle a fake exit code. The markers
        // below are random per call, and any command reaching for the
        // protocol namespace is refused outright.
        // `anyhow`-style `{err:#}` keeps the full error chain (the `String`
        // boundary signature is kept so `ToolResult` callers are untouched).
        let protocol = self.build_protocol(command)?;
        // Captured up front so the fallback below never has to block on the
        // mutex from inside the async context.
        let known_cwd = self.cwd.lock().await.clone();
        let sentinel = protocol.sentinel.clone();
        let start_marker = protocol.start_marker.clone();
        let cwd_marker = protocol.cwd_marker.clone();
        let status_path = protocol.status_path.clone();

        // Git Bash on Windows can swallow PTY input that arrives before its
        // login profile has finished and the first prompt is on screen. Wait
        // for that initial output before writing the first command. A shell
        // that never prints a prompt still proceeds after the bounded wait.
        self.ensure_warmed(max_ms.min(10_000)).await;

        {
            let mut writer = self.writer.lock().await;
            if let Err(err) = writer.write_all(protocol.script.as_bytes()) {
                return Err(format!("shell write failed: {err:#}"));
            }
            writer
                .flush()
                .map_err(|e| format!("shell flush failed: {e:#}"))?;
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(max_ms);
        let done = loop {
            // The shell writes the sentinel and cwd marker separately. A PTY
            // reader may observe the sentinel first; draining at that point
            // loses the live cwd and lets the next call trust a stale value.
            let complete = {
                let buffer = self.output.lock().await;
                let text = String::from_utf8_lossy(&buffer);
                find_line(text.as_ref(), &sentinel).is_some()
                    && cwd_from_transcript(text.as_ref(), &sentinel, &cwd_marker).is_some()
            };
            if complete {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        if !done {
            // Drain whatever arrived so a timeout does not corrupt the next
            // call. Keep a short tail for the error: on Windows CI a marker
            // timeout without the transcript is almost impossible to diagnose.
            let tail = {
                let mut guard = self.output.lock().await;
                let text = String::from_utf8_lossy(&guard);
                let tail = diagnostic_tail(text.as_ref(), 2_000);
                guard.clear();
                tail
            };
            return Err(if tail.is_empty() {
                format!("timed out after {max_ms} ms")
            } else {
                format!("timed out after {max_ms} ms; last shell output: {tail:?}")
            });
        }

        // Take the buffer and reset it in one lock: the reader thread keeps
        // appending, so the only thing that makes this well-defined is doing the
        // read-and-clear atomically.
        let buffer = {
            let mut guard = self.output.lock().await;
            std::mem::take(&mut *guard)
        };
        let text = String::from_utf8_lossy(&buffer).to_string();

        // Everything after the marker up to the cwd probe is protocol, not
        // output. Concatenate the tail before parsing: ConPTY can wrap a long
        // PowerShell prompt/path across lines, which made the old line-based
        // parser wait forever for a cwd marker that was present but split.
        let cwd = cwd_from_transcript(&text, &sentinel, &cwd_marker).unwrap_or(known_cwd);
        *self.cwd.lock().await = cwd.clone();

        // Real output lives between the start and done markers. The PTY echo
        // of the script (including `__mixd_rc=$?` and the `printf` lines) falls
        // outside that window, so it can no longer leak into the result.
        let mut out = slice_output(&text, &start_marker, &sentinel);
        while out.ends_with('\n') || out.ends_with('\r') {
            out.pop();
        }

        let exit_code = read_exit_code(&status_path);
        let _ = std::fs::remove_file(&status_path);
        Ok((truncate_output(&out), exit_code, cwd))
    }
}
