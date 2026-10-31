//! Native model operations for the local runtimes.
//!
//! Discovery answers "what is installed and what can it serve". This module
//! answers "make the runtime do something": pull, delete, load, unload. The
//! wire/CLI details stay here so the Tauri layer only has to manage jobs and
//! events.
//!
//! Failures are encoded with [`crate::local_error`] so the UI can translate
//! them; the technical detail stays as the last argument.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

use crate::discovery::{find_on_path, origin_of};
use crate::local_error::{error, simple};

/// One progress step from a pull stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullProgress {
    pub status: String,
    pub completed: u64,
    pub total: u64,
}

impl PullProgress {
    pub fn is_finished(&self) -> bool {
        self.status == "success"
    }
}

/// Cooperative cancellation that wakes a blocked stream immediately.
///
/// Polling an `AtomicBool` only on each received chunk meant a stalled download
/// ignored Stop for the whole 600 s read timeout. `Notify` stores a permit when
/// `cancel()` runs before anyone is waiting, so no wake-up is lost.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.notify.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Resolve as soon as `cancel()` has been called, including when it was
    /// called before this future was awaited.
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            self.notify.notified().await;
        }
    }
}

/// The local-model HTTP client.
///
/// Every operation used to build its own `Client`, rebuilding the pool for a
/// request that always goes to `127.0.0.1`. `no_proxy` is deliberate: a system
/// proxy must not intercept a loopback model server.
fn local_client() -> anyhow::Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    let built = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // A total timeout would cut a long but healthy download at the
            // deadline; a read timeout only fires when the peer goes silent.
            .read_timeout(Duration::from_secs(600))
            .no_proxy()
            .build()
            .map_err(|err| format!("could not initialise the local HTTP client: {err}"))
    });
    built.as_ref().map_err(|err| anyhow::anyhow!(err.clone()))
}

/// `POST /api/pull` on Ollama, forwarding each NDJSON line to `on_progress`.
///
/// Cancellation is cooperative: the manager sets the flag, and the next chunk
/// or line stops the stream. Dropping the response is what tells Ollama to
/// stop sending; Ollama itself does not have a separate cancel endpoint.
pub async fn ollama_pull(
    origin: &str,
    model: &str,
    cancel: CancelToken,
    mut on_progress: impl FnMut(PullProgress),
) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        return Err(simple("pull_cancelled"));
    }
    let url = format!("{}/api/pull", origin_of(origin));
    let response = local_client()?
        .post(&url)
        .json(&serde_json::json!({ "name": model, "stream": true }))
        .send()
        .await
        .map_err(|err| error("request_failed", &[url.clone(), err.to_string()]))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(error(
            "http_status",
            &[status.to_string(), body.trim().to_string()],
        ));
    }

    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut saw_success = false;
    loop {
        // Race the stream against cancellation: a silent server must not make
        // Stop wait for the transport's read timeout.
        let chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(simple("pull_cancelled")),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|err| error("pull_failed", &[format!("stream: {err}")]))?;
        buffer.extend_from_slice(&chunk);
        while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(progress) = pull_progress(&line)? {
                if progress.is_finished() {
                    saw_success = true;
                }
                on_progress(progress);
            }
        }
        if saw_success {
            return Ok(());
        }
    }
    // Ollama does not guarantee a trailing newline, so the terminal `success`
    // line can still be sitting in the buffer when the stream closes.
    let tail = String::from_utf8_lossy(&buffer);
    if let Some(progress) = pull_progress(&tail)? {
        if progress.is_finished() {
            saw_success = true;
        }
        on_progress(progress);
    }
    if saw_success {
        return Ok(());
    }
    // An EOF without the explicit `success` line is a truncated pull: treating
    // it as success would leave a half-downloaded tag that looks usable.
    if cancel.is_cancelled() {
        return Err(simple("pull_cancelled"));
    }
    Err(error(
        "pull_failed",
        &["stream ended before the download completed".to_string()],
    ))
}

/// Parse one NDJSON line from `/api/pull`. Blank/non-JSON lines are ignored;
/// an `error` field becomes a coded failure.
fn pull_progress(line: &str) -> anyhow::Result<Option<PullProgress>> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if let Some(message) = value.get("error").and_then(serde_json::Value::as_str) {
        return Err(error("pull_failed", &[message.trim().to_string()]));
    }
    Ok(Some(PullProgress {
        status: value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("pulling")
            .to_string(),
        completed: value
            .get("completed")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        total: value
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    }))
}

async fn ollama_json(
    method: reqwest::Method,
    origin: &str,
    path: &str,
    body: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let url = format!("{}{}", origin_of(origin), path);
    let response = local_client()?
        .request(method, &url)
        .json(&body)
        .send()
        .await
        .map_err(|err| error("request_failed", &[url.clone(), err.to_string()]))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(error(
            "http_status",
            &[status.to_string(), text.trim().to_string()],
        ));
    }
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(&text).map_err(|err| error("invalid_json", &[url, err.to_string()]))
}

/// Delete one Ollama model.
pub async fn ollama_delete(origin: &str, model: &str) -> anyhow::Result<()> {
    ollama_json(
        reqwest::Method::DELETE,
        origin,
        "/api/delete",
        serde_json::json!({ "name": model }),
    )
    .await
    .map(|_| ())
}

/// Load a model (or refresh its keep-alive) with an empty, one-token request.
pub async fn ollama_load(origin: &str, model: &str, keep_alive: &str) -> anyhow::Result<()> {
    ollama_json(
        reqwest::Method::POST,
        origin,
        "/api/generate",
        serde_json::json!({
            "model": model,
            "prompt": "",
            "stream": false,
            "keep_alive": keep_alive,
            "options": { "num_predict": 1 },
        }),
    )
    .await
    .map(|_| ())
}

/// Unload a model immediately.
pub async fn ollama_unload(origin: &str, model: &str) -> anyhow::Result<()> {
    ollama_load(origin, model, "0").await
}

fn lms_program() -> anyhow::Result<PathBuf> {
    find_on_path("lms").ok_or_else(|| simple("lms_missing"))
}

async fn run_lms(args: &[&str]) -> anyhow::Result<String> {
    let program = lms_program()?;
    let output = tokio::process::Command::new(&program)
        .args(args)
        .output()
        .await
        .map_err(|err| {
            error(
                "lms_spawn_failed",
                &[program.display().to_string(), err.to_string()],
            )
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(error(
            "lms_failed",
            &[
                args.join(" "),
                if stderr.is_empty() { stdout } else { stderr },
            ],
        ));
    }
    Ok(stdout)
}

pub async fn lmstudio_start_server(port: u16) -> anyhow::Result<()> {
    run_lms(&["server", "start", "--port", &port.to_string()])
        .await
        .map(|_| ())
}

pub async fn lmstudio_stop_server() -> anyhow::Result<()> {
    run_lms(&["server", "stop"]).await.map(|_| ())
}

/// `lms get -y <model>`, cancellable.
///
/// `output()` waits for the process to finish, which made a multi-gigabyte
/// download impossible to cancel. Spawning and polling the cancellation flag
/// lets the job stop the CLI instead of only changing a label in the UI. Both
/// pipes are drained concurrently so a chatty progress bar cannot fill the OS
/// buffer and deadlock the child.
pub async fn lmstudio_pull(model: &str, cancel: CancelToken) -> anyhow::Result<()> {
    let program = lms_program()?;
    let mut child = tokio::process::Command::new(&program)
        .args(["get", "-y", model])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| {
            error(
                "lms_spawn_failed",
                &[program.display().to_string(), err.to_string()],
            )
        })?;
    let stdout = child.stdout.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer).await;
            buffer
        })
    });
    let stderr = child.stderr.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer).await;
            buffer
        })
    });

    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(simple("pull_cancelled"));
        }
        status = child.wait() => status,
    };
    let status =
        status.map_err(|err| error("lms_failed", &["get".to_string(), err.to_string()]))?;
    let stdout = join_output(stdout).await;
    let stderr = join_output(stderr).await;
    if !status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(error("lms_failed", &["get".to_string(), detail]));
    }
    Ok(())
}

async fn join_output(task: Option<tokio::task::JoinHandle<Vec<u8>>>) -> String {
    match task {
        Some(task) => task
            .await
            .map(|buffer| String::from_utf8_lossy(&buffer).to_string())
            .unwrap_or_default(),
        None => String::new(),
    }
}

pub async fn lmstudio_load(model: &str) -> anyhow::Result<()> {
    run_lms(&["load", "-y", model]).await.map(|_| ())
}

pub async fn lmstudio_unload(model: &str) -> anyhow::Result<()> {
    run_lms(&["unload", model]).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_progress_recognises_the_terminal_status() {
        assert!(PullProgress {
            status: "success".into(),
            ..PullProgress::default()
        }
        .is_finished());
        assert!(!PullProgress {
            status: "downloading".into(),
            completed: 1,
            total: 2,
        }
        .is_finished());
    }

    #[test]
    fn error_codes_carry_arguments_for_the_ui() {
        let err = simple("pull_cancelled");
        assert_eq!(format!("{err:#}"), "mix-local-error:pull_cancelled|[]");
        let err = error("lms_failed", &["get".into(), "denied".into()]);
        assert!(format!("{err:#}").starts_with("mix-local-error:lms_failed|[\"get\""));
    }

    #[tokio::test]
    async fn a_cancel_before_the_wait_still_wakes_it() {
        let token = CancelToken::new();
        token.cancel();
        assert!(token.is_cancelled());
        tokio::time::timeout(std::time::Duration::from_millis(100), token.cancelled())
            .await
            .expect("an already-cancelled token must resolve immediately");
    }

    #[test]
    fn pull_lines_without_a_trailing_newline_still_count() {
        let progress = pull_progress(r#"{"status":"success"}"#).expect("parse");
        assert!(progress.expect("progress").is_finished());
        assert!(pull_progress("").expect("blank").is_none());
        assert!(pull_progress("not json").expect("garbage").is_none());
        let err = pull_progress(r#"{"error":"manifest unknown"}"#).expect_err("error line");
        assert!(format!("{err:#}").contains("pull_failed"));
    }
}
