use super::*;

impl ShellSession {
    /// Spawn a login shell in `cwd`, or return `None` when no PTY is available.
    pub(crate) fn spawn(
        cwd: &str,
        id: &str,
        sandbox_root: Option<&std::path::Path>,
    ) -> Option<Arc<Self>> {
        let config = detect_shell();
        let config = match sandbox_root {
            Some(root) => sandboxed_shell_config(&config, root).unwrap_or_else(|| {
                tracing::warn!(
                    "terminal sandbox is requested but unavailable on this platform; \
                     the shell will run unsandboxed"
                );
                config
            }),
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
    pub(crate) fn kill(&self) {
        let mut killer = match self.killer.lock() {
            Ok(k) => k,
            Err(_) => return,
        };
        if let Err(err) = killer.kill() {
            tracing::debug!(session = %self.id, %err, "shell kill failed");
        }
    }

    pub(crate) async fn cwd(&self) -> String {
        self.cwd.lock().await.clone()
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
            // PowerShell in raw PTY mode handles CRLF reliably for interactive evaluation
            format!(
                "Write-Output \"`n{start}\"\r\n{command}\r\n$__mixd_rc = if ($LASTEXITCODE -ne $null) {{ $LASTEXITCODE }} elseif ($?) {{ 0 }} else {{ 1 }}\r\nif ($__mixd_rc -eq $null) {{ $__mixd_rc = 0 }}\r\n[System.IO.File]::WriteAllText('{status}', \"$__mixd_rc\", [System.Text.Encoding]::ASCII)\r\nWrite-Output \"`n{sentinel}\"\r\nWrite-Output \"{cwd_marker}$($ExecutionContext.SessionState.Path.CurrentLocation.Path)__\"\r\n",
                start = start_marker,
                status = status_str.replace('\'', "''"),
                sentinel = sentinel,
                cwd_marker = cwd_marker,
            )
        };

        {
            let mut writer = self.writer.lock().await;
            if let Err(err) = writer.write_all(script.as_bytes()) {
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
