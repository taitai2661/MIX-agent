//! `mixd serve` — minimal HTTP + SSE bridge.
//!
//!雛形 (A7/P3-1): the Tauri command surface is the canonical API; this module
//! exposes the smallest subset a phone PWA needs over plain HTTP so a
//! `WebView`-less headless process (`serve.rs`) can run on a NAS and approve
//! Runs from a phone:
//!
//! * `GET /health` (no auth — liveness only)
//! * `GET /api/v1/runs?limit=` — recent runs for inbox context
//! * `GET /api/v1/runs/{id}` — single run header (no transcript; use SSE)
//! * `GET /api/v1/approvals/pending` — engine `list_pending()`
//! * `POST /api/v1/approvals/{id}/decision` — same guards as
//!   `commands::approvals::decide_approval`
//! * `POST /api/v1/answers` — answer an `ask_user` card (`question_requested`)
//!   via the same `format_question_answer` + `steer` path as the desktop UI
//! * `POST /api/v1/runs/{id}/cancel` — signal cancellation for a live Run
//!   (the engine finishes it as `cancelled`; no-op when already finished)
//! * `GET /api/v1/runs/{id}/events` — recent replay (`event: replay`, up to
//!   1000 rows via `load_replay_window`) followed by live engine broadcast as
//!   SSE (`event: activity` + 15 s heartbeats + `event: done`)
//! * `GET /api/v1/scheduled-jobs` — Desktop `scheduled_jobs` overview
//!
//! Compatibility notes:
//!
//! * Route shapes follow the Docker edition (`routes_runs.py`,
//!   `routes_schedules.py`) where they already exist; the error envelope is
//!   `{ "error": "<message>" }` rather than the Docker `{ detail, code }`
//!   shape because Desktop errors are plain strings (`error::cmd_err`).
//! * The SSE stream replays up to 1000 recent events, then streams live.
//!   Phone clients still call `GET /approvals/pending` on (re)connect to cover
//!   prompts, but the transcript gap is now closed.
//!
//! Deliberately dependency-free: only `tokio` (`net`/`time`/`io-util`) and
//! `serde_json`. If this grows beyond the approval inbox, migrate to axum
//! (TODO) rather than extending the hand-rolled parser below.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use mixd_core::runs::engine::RunEngine;
use mixd_storage::Storage;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

/// Bound a single request head+body read. A slow client that dribbles bytes
/// forever (slowloris) otherwise pins a task and a socket indefinitely.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Total simultaneous connections, including long-lived SSE streams.
const MAX_CONNECTIONS: usize = 64;
/// SSE streams are long-lived by design; only *silence* closes one. Heartbeats
/// every 15 s keep a healthy stream well inside this idle budget.
const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// A stalled TCP peer must not pin an SSE task forever on a write.
const SSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared state for the serve loop. A narrow struct (not `AppState`) so the
/// headless process does not need a browser, MCP host or WebView.
#[derive(Clone)]
pub struct ServeState {
    pub storage: Arc<Storage>,
    pub engine: Arc<RunEngine>,
    /// Expected `Authorization: Bearer <token>`. There is no anonymous mode:
    /// an empty token denies every `/api/v1` request, loopback included.
    pub token: String,
    pub version: String,
    /// Bare `scheme://host[:port]` origins a browser may call from. Empty means
    /// reject every `Origin` header (non-browser clients send none).
    pub allowed_origins: Vec<String>,
    /// Exact `Host` header values (lowercased) accepted for this bind. Empty
    /// disables the Host check (a wildcard bind cannot know its public name).
    pub allowed_hosts: Vec<String>,
}

fn header<'a>(headers: &'a [(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| *value)
}

fn token_matches(token: &str, candidate: &str) -> bool {
    // Constant-time-ish comparison without pulling in another crate: the
    // token is a bearer secret, so avoid short-circuiting on length only.
    !token.is_empty()
        && candidate.len() == token.len()
        && candidate
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn bearer_allowed(token: &str, headers: &[(&str, &str)]) -> bool {
    let Some(auth) = header(headers, "authorization") else {
        return false;
    };
    // RFC 7235: auth schemes are case-insensitive. Compare the 7-byte
    // "Bearer " prefix without allocation, then pass the credential to the
    // constant-time comparison.
    let lower = auth.to_ascii_lowercase();
    let bearer = lower
        .strip_prefix("bearer ")
        .map(|_| &auth[7..])
        .unwrap_or_default();
    token_matches(token, bearer)
}

/// Defeat DNS rebinding: a browser must arrive with a Host we bound to and
/// (if it sent one) an Origin we configured. A plain `curl` sends no Origin
/// and still works with the bearer token.
fn target_allowed(
    headers: &[(&str, &str)],
    allowed_origins: &[String],
    allowed_hosts: &[String],
) -> bool {
    if !allowed_hosts.is_empty() {
        let Some(host) = header(headers, "host") else {
            return false;
        };
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if !allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&host))
        {
            return false;
        }
    }
    if let Some(origin) = header(headers, "origin") {
        let origin = origin.trim().trim_end_matches('/').to_ascii_lowercase();
        if origin == "null" {
            return false;
        }
        return allowed_origins
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&origin));
    }
    true
}

impl ServeState {
    fn authorized(&self, headers: &[(&str, &str)]) -> bool {
        bearer_allowed(&self.token, headers)
    }

    /// `EventSource` cannot send headers, so only the SSE route accepts
    /// `?token=` as a fallback. Never log this raw path — use `redact_path`.
    fn authorized_query(&self, full_path: &str) -> bool {
        query_param(full_path, "token").is_some_and(|t| token_matches(&self.token, &t))
    }

    fn request_target_allowed(&self, headers: &[(&str, &str)]) -> bool {
        target_allowed(headers, &self.allowed_origins, &self.allowed_hosts)
    }
}

/// Strip the bearer secret from a request target before it can reach a log.
fn redact_path(full: &str) -> String {
    let mut out = String::with_capacity(full.len());
    let mut rest = full;
    while let Some(pos) = rest.find("token=") {
        out.push_str(&rest[..pos + "token=".len()]);
        out.push_str("<redacted>");
        rest = &rest[pos + "token=".len()..];
        let end = rest.find(['&', '#']).unwrap_or(rest.len());
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Read one HTTP/1.1 request head plus `Content-Length` body. Caps the head at
/// 16 KiB and the body at 256 KiB so a phone on a bad network cannot make the
/// NAS box buffer unboundedly.
async fn read_request(
    reader: &mut (impl AsyncBufReadExt + Unpin),
) -> anyhow::Result<Option<Request>> {
    let mut head = Vec::with_capacity(1024);
    loop {
        let mut line = Vec::new();
        let n = reader.read_until(b'\n', &mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        head.extend_from_slice(&line);
        if head.len() > 16 * 1024 {
            anyhow::bail!("request head too large");
        }
        if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
            break;
        }
    }
    let head_text = String::from_utf8_lossy(&head);
    let mut lines = head_text.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        let line = line.trim_end_matches('\r');
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_string();
        let value = value.trim().to_string();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().unwrap_or(0).min(256 * 1024);
        }
        headers.push((name, value));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }
    // Query strings stay on the path for the router; strip them per-route.
    Ok(Some(Request {
        method,
        path: target,
        headers,
        body,
    }))
}

fn json_response(status: u16, value: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    format!(
        "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        reason(status),
        body.len(),
        status = status,
    )
    .into_bytes()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        408 => "Request Timeout",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

fn err(status: u16, message: impl Into<String>) -> Vec<u8> {
    json_response(status, &serde_json::json!({ "error": message.into() }))
}

fn path_only(full: &str) -> &str {
    full.split(['?', '#']).next().unwrap_or(full)
}

fn query_param(full: &str, key: &str) -> Option<String> {
    full.split(['?', '&', '#']).skip(1).find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

async fn handle(state: &ServeState, req: Request) -> Vec<u8> {
    let path = path_only(&req.path);
    if req.method == "GET" && path == "/health" {
        return json_response(
            200,
            &serde_json::json!({ "ok": true, "version": state.version }),
        );
    }
    let headers: Vec<(&str, &str)> = req
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    // Host/Origin first: a rebinding browser must not even get a token check
    // that could be used as an oracle. The token is required on loopback too.
    if !state.request_target_allowed(&headers) {
        return err(403, "host or origin not allowed");
    }
    if !state.authorized(&headers) {
        return err(401, "missing or invalid bearer token");
    }
    match (req.method.as_str(), path) {
        ("GET", "/api/v1/runs") => runs_list(state, &req.path).await,
        ("GET", p)
            if p.starts_with("/api/v1/runs/")
                && !p.ends_with("/events")
                && !p.ends_with("/cancel") =>
        {
            run_detail(state, p).await
        }
        ("GET", "/api/v1/approvals/pending") => approvals_pending(state).await,
        ("POST", p) if p.starts_with("/api/v1/approvals/") && p.ends_with("/decision") => {
            let id = p
                .strip_prefix("/api/v1/approvals/")
                .and_then(|s| s.strip_suffix("/decision"))
                .unwrap_or_default();
            decide(state, id, &req.body).await
        }
        // S2+A7 bridge: answer a structured `ask_user` card from the phone.
        // Mirrors `commands::approvals::answer_question` (steer + retire).
        ("POST", "/api/v1/answers") => answer(state, &req.body).await,
        ("POST", p) if p.starts_with("/api/v1/runs/") && p.ends_with("/cancel") => {
            let id = p
                .strip_prefix("/api/v1/runs/")
                .and_then(|s| s.strip_suffix("/cancel"))
                .unwrap_or_default();
            cancel_run(state, id).await
        }
        // Explicit stub: creating a Run needs the full Tauri `AppState`
        // (`dispatch` 815 lines). Returning 501 (not 404) so phone clients
        // can distinguish "not yet implemented" from "wrong URL".
        ("POST", "/api/v1/runs") => err(
            501,
            "run creation is not yet served; approve/cancel/answer existing runs",
        ),
        ("GET", "/api/v1/scheduled-jobs") => scheduled_jobs(state).await,
        // NOTE: `/api/v1/runs/{id}/events` (SSE) never reaches `handle`: it
        // needs the open socket, so `serve_connection` routes it beforehand.
        ("GET" | "POST", _) => err(404, "not found"),
        _ => err(405, "method not allowed"),
    }
}

async fn runs_list(state: &ServeState, full_path: &str) -> Vec<u8> {
    let limit: i64 = query_param(full_path, "limit")
        .and_then(|v| v.parse().ok())
        .map(|n: i64| n.clamp(1, 100))
        .unwrap_or(20);
    let rows: Vec<(String, Option<String>, String)> =
        sqlx::query_as("SELECT id, conversation_id, status FROM runs ORDER BY rowid DESC LIMIT ?")
            .bind(limit)
            .fetch_all(state.storage.pool())
            .await
            .unwrap_or_default();
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, conversation_id, status)| {
            serde_json::json!({ "id": id, "conversation_id": conversation_id, "status": status })
        })
        .collect();
    json_response(200, &serde_json::json!({ "runs": items }))
}

/// Single run header for the phone inbox. No transcript (use SSE `replay`
/// for that); unknown ids are 404 so the inbox can drop stale cards.
async fn run_detail(state: &ServeState, full_path: &str) -> Vec<u8> {
    let id = full_path
        .split('?')
        .next()
        .unwrap_or_default()
        .strip_prefix("/api/v1/runs/")
        .unwrap_or_default();
    if id.is_empty() || id.contains('/') {
        return err(404, "unknown run");
    }
    let row: Option<(String, Option<String>, String, String)> =
        sqlx::query_as("SELECT id, conversation_id, status, data FROM runs WHERE id = ?")
            .bind(id)
            .fetch_optional(state.storage.pool())
            .await
            .unwrap_or(None);
    let Some((run_id, conversation_id, status, data)) = row else {
        return err(404, "unknown run");
    };
    let meta: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
    json_response(
        200,
        &serde_json::json!({
            "id": run_id,
            "conversation_id": conversation_id,
            "status": status,
            "mode": meta.get("mode"),
            "modelId": meta.get("modelId"),
            "updatedAt": meta.get("updatedAt"),
        }),
    )
}

async fn approvals_pending(state: &ServeState) -> Vec<u8> {
    // Same source as `commands::tools::list_pending_approvals`, minus the
    // conversation-title join: the phone inbox resolves titles lazily through
    // `/api/v1/runs` so one pending prompt never costs a SQL join per poll.
    let pending = state.engine.executor.list_pending().await;
    let items: Vec<serde_json::Value> = pending
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "approval_id": p.approval_id,
                "run_id": p.run_id.to_string(),
                "tool_call_id": p.tool_call_id,
                "tool_name": p.tool_name,
                "arguments": p.arguments,
                "created_at": p.created_at.to_rfc3339(),
                "expires_at": p.expires_at.to_rfc3339(),
                "risk": p.risk,
            })
        })
        .collect();
    json_response(200, &serde_json::json!({ "approvals": items }))
}

/// Read a boolean field from either snake_case or camelCase.
///
/// The phone client is being migrated to snake_case; accepting both during
/// the transition keeps older builds working without weakening any guard.
fn bool_alias(input: &serde_json::Value, snake: &str, camel: &str) -> bool {
    input
        .get(snake)
        .and_then(|v| v.as_bool())
        .or_else(|| input.get(camel).and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

async fn decide(state: &ServeState, id: &str, body: &[u8]) -> Vec<u8> {
    if id.is_empty() {
        return err(404, "unknown or expired approval");
    }
    let input: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return err(422, "body must be JSON"),
    };
    let grant = bool_alias(&input, "grant", "grant");
    let remember = bool_alias(&input, "remember", "remember");
    let remember_session = bool_alias(&input, "remember_session", "rememberSession");
    let acknowledge_risk = bool_alias(&input, "acknowledge_risk", "acknowledgeRisk");

    // Mirror `commands::approvals::decide_approval` so the phone cannot grant
    // what the desktop UI would refuse.
    let pending = state.engine.executor.list_pending().await;
    let Some(prompt) = pending.iter().find(|p| p.approval_id == id) else {
        return err(404, format!("unknown or expired approval {id}"));
    };
    let dangerous = prompt
        .risk
        .as_ref()
        .is_some_and(mixd_tools::risk::RiskAssessment::is_dangerous);
    if dangerous {
        if grant && (remember || remember_session) {
            return err(
                422,
                "dangerous commands cannot be remembered; approve this one call explicitly",
            );
        }
        if grant && !acknowledge_risk {
            return err(422, "dangerous command requires explicit acknowledgement");
        }
    }
    if remember && !grant {
        return err(422, "remember requires an approval grant");
    }
    let Some(tool_name) = state.engine.executor.tool_name_for_approval(id).await else {
        return err(404, format!("unknown approval {id}"));
    };
    let outcome = if grant {
        mixd_tools::approval::ApprovalOutcome::Approve {
            remember,
            remember_session,
        }
    } else {
        mixd_tools::approval::ApprovalOutcome::Deny
    };
    match state
        .engine
        .executor
        .resolve_approval_checked(id, &tool_name, outcome, acknowledge_risk)
        .await
    {
        Ok(()) => json_response(200, &serde_json::json!({ "ok": true })),
        Err(e) => err(500, format!("could not resolve approval: {e}")),
    }
}

/// Is `question_id` still awaiting an answer on this run?
///
/// The engine records `question_requested` / `question_resolved` as durable
/// run events. Requiring a matching, still-open event stops a stale or forged
/// POST from steering a live run with arbitrary text. A direct SQL query (not
/// a truncated replay window) keeps this correct for long transcripts.
async fn question_is_pending(state: &ServeState, run_id: uuid::Uuid, question_id: &str) -> bool {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT kind FROM run_events \
         WHERE run_id = ?1 AND json_extract(data, '$.question_id') = ?2 \
         ORDER BY sequence ASC",
    )
    .bind(run_id.to_string())
    .bind(question_id)
    .fetch_all(state.storage.pool())
    .await
    .unwrap_or_default();
    let mut pending = false;
    for (kind,) in rows {
        if kind == "question_requested" {
            pending = true;
        } else if kind == "question_resolved" {
            pending = false;
        }
    }
    pending
}

/// Answer a structured `ask_user` card (`question_requested`) from the phone.
///
/// Body: `{ "run_id": "<uuid>", "question_id": "<id>", "selected": [...],
/// "free_text": "<note>" }`. Mirrors `answer_question`: formats via
/// `format_question_answer`, steers the Run, and always emits
/// `question_resolved` so the card retires even when the Run already ended.
async fn answer(state: &ServeState, body: &[u8]) -> Vec<u8> {
    let input: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return err(422, "body must be JSON"),
    };
    let run_id_raw = input.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = match uuid::Uuid::parse_str(run_id_raw) {
        Ok(id) => id,
        Err(_) => return err(422, "run_id must be a UUID"),
    };
    let selected: Vec<String> = input
        .get("selected")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let free_text = input.get("free_text").and_then(|v| v.as_str());
    let text = match mixd_tools::approval::format_question_answer(&selected, free_text) {
        Ok(t) => t,
        Err(e) => return err(422, e),
    };
    let question_id = match input
        .get("question_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(id) => id.to_string(),
        None => return err(422, "question_id is required"),
    };
    if !question_is_pending(state, run_id, &question_id).await {
        return err(409, "no pending question with that id for this run");
    }
    let accepted = state.engine.steer(run_id, text).await;
    state
        .engine
        .emit(
            run_id,
            "question_resolved",
            serde_json::json!({ "question_id": question_id, "accepted": accepted }),
        )
        .await;
    json_response(
        200,
        &serde_json::json!({ "ok": true, "accepted": accepted }),
    )
}

/// Signal cancellation for a live Run.
///
/// Mirrors the desktop Stop button: notifies the engine's cancellation handle;
/// the engine records the terminal `cancelled` status. Unknown ids and
/// already-finished Runs are still `ok: true` (idempotent).
async fn cancel_run(state: &ServeState, id: &str) -> Vec<u8> {
    let run_id = match uuid::Uuid::parse_str(id) {
        Ok(run_id) => run_id,
        Err(_) => return err(404, "unknown run"),
    };
    state.engine.cancel(run_id).await;
    json_response(200, &serde_json::json!({ "ok": true }))
}

async fn scheduled_jobs(state: &ServeState) -> Vec<u8> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, data FROM scheduled_jobs ORDER BY id")
            .fetch_all(state.storage.pool())
            .await
            .unwrap_or_default();
    let mut items = Vec::with_capacity(rows.len());
    for (id, raw) in rows {
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let get = |k: &str| {
            data.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let cron = get("cron");
        let next_run_at = mixd_core::schedule::CronSchedule::parse(&cron)
            .ok()
            .and_then(|s| s.next_after(chrono::Utc::now()))
            .map(|t| t.to_rfc3339());
        items.push(serde_json::json!({
            "id": id,
            "name": data.get("name").and_then(|v| v.as_str()).unwrap_or("(unnamed)"),
            "cron": cron,
            "enabled": data.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            "prompt": get("prompt"),
            "mode": data.get("mode").and_then(|v| v.as_str()).unwrap_or("chat"),
            "next_run_at": next_run_at,
        }));
    }
    json_response(200, &serde_json::json!({ "jobs": items }))
}

/// Write one SSE frame with a bounded wait. A client that stops reading must
/// not hold a connection (and its semaphore permit) forever.
async fn sse_write(stream: &mut (impl AsyncWriteExt + Unpin), bytes: &[u8]) -> bool {
    tokio::time::timeout(SSE_WRITE_TIMEOUT, stream.write_all(bytes))
        .await
        .is_ok()
}

/// Stream live engine events for one run as SSE. Replays recent history first
/// (so a reconnecting phone does not miss what happened while it was away),
/// then streams live broadcasts until the terminal status.
async fn serve_sse(
    state: &ServeState,
    stream: &mut (impl AsyncWriteExt + Unpin),
    run_id: &str,
) -> anyhow::Result<()> {
    let mut receiver = state.engine.subscribe();
    if !sse_write(
        stream,
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: keep-alive\r\nx-accel-buffering: no\r\n\r\n: ready\n\n",
    )
    .await
    {
        return Ok(());
    }
    let wanted = run_id.to_string();
    // Replay recent history first (best-effort): a reconnecting phone reads
    // `pending` on connect, but the transcript in between would otherwise be
    // lost. Failures here must not break the live stream.
    if let Ok(window) =
        mixd_core::runs::events::load_replay_window(state.storage.pool(), &wanted, 1000).await
    {
        for (replay_run_id, sequence, kind, data, created_at) in window.rows {
            let data_value: serde_json::Value =
                serde_json::from_str(&data).unwrap_or(serde_json::Value::Null);
            let payload = serde_json::json!({
                "run_id": replay_run_id,
                "sequence": sequence,
                "kind": kind,
                "data": data_value,
                "created_at": created_at,
            });
            let frame = format!(
                "event: replay\ndata: {}\n\n",
                serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into())
            );
            if !sse_write(stream, frame.as_bytes()).await {
                return Ok(());
            }
        }
        if stream.flush().await.is_err() {
            return Ok(());
        }
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
    // Never hold the heartbeat tick across an await that can fail: `tick`
    // borrows the interval mutably and `write_all` below can return early.
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Idle, not total, timeout: a quiet Run keeps producing heartbeats, so a
    // healthy stream can stay open for hours. A dead phone gets reaped instead
    // of holding a connection permit forever.
    let idle = tokio::time::sleep(SSE_IDLE_TIMEOUT);
    tokio::pin!(idle);
    loop {
        tokio::select! {
            _ = idle.as_mut() => {
                tracing::debug!(run_id = %wanted, "sse: idle timeout");
                return Ok(());
            }
            _ = heartbeat.tick() => {
                if !sse_write(stream, b": heartbeat\n\n").await {
                    return Ok(());
                }
                if stream.flush().await.is_err() {
                    return Ok(());
                }
                idle.as_mut().reset(tokio::time::Instant::now() + SSE_IDLE_TIMEOUT);
            }
            result = receiver.recv() => {
                let event = match result {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                };
                if event.run_id.to_string() != wanted {
                    continue;
                }
                let payload = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
                let frame = format!("event: activity\ndata: {payload}\n\n");
                if !sse_write(stream, frame.as_bytes()).await {
                    return Ok(());
                }
                if stream.flush().await.is_err() {
                    return Ok(());
                }
                idle.as_mut().reset(tokio::time::Instant::now() + SSE_IDLE_TIMEOUT);
                let terminal = event.kind == "status"
                    && event
                        .data
                        .get("status")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| matches!(s, "completed" | "failed" | "cancelled" | "interrupted"));
                if terminal {
                    let _ = sse_write(stream, b"event: done\ndata: {}\n\n").await;
                    let _ = stream.flush().await;
                    return Ok(());
                }
            }
        }
    }
}

/// Serve one TCP connection: exactly one request, except SSE streams which
/// hold the socket until the run ends or the phone hangs up.
async fn serve_connection(state: ServeState, mut socket: tokio::net::TcpStream) {
    let (reader_half, mut writer_half) = socket.split();
    let mut reader = tokio::io::BufReader::new(reader_half);
    let req = match tokio::time::timeout(REQUEST_READ_TIMEOUT, read_request(&mut reader)).await {
        Ok(Ok(Some(req))) => req,
        Ok(Ok(None)) => return,
        Ok(Err(_)) => {
            let _ = writer_half.write_all(&err(400, "unreadable request")).await;
            return;
        }
        Err(_) => {
            // Slowloris: the peer stopped sending before completing the head.
            let _ = writer_half.write_all(&err(408, "request timed out")).await;
            return;
        }
    };
    tracing::debug!(path = %redact_path(&req.path), "serve: request");
    let path = path_only(&req.path).to_string();
    let is_sse =
        req.method == "GET" && path.starts_with("/api/v1/runs/") && path.ends_with("/events");
    if is_sse {
        let headers: Vec<(&str, &str)> = req
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if !state.request_target_allowed(&headers) {
            let _ = writer_half
                .write_all(&err(403, "host or origin not allowed"))
                .await;
            return;
        }
        // Header auth first, `?token=` fallback for `EventSource` (§4.1).
        // The raw path (with its token) is never logged; see `redact_path`.
        if !state.authorized(&headers) && !state.authorized_query(&req.path) {
            let _ = writer_half
                .write_all(&err(401, "missing or invalid bearer token"))
                .await;
            return;
        }
        let run_id = path
            .strip_prefix("/api/v1/runs/")
            .and_then(|s| s.strip_suffix("/events"))
            .unwrap_or_default()
            .to_string();
        // Reunite the halves: SSE needs a single writable socket.
        drop(reader);
        let _ = serve_sse(&state, &mut writer_half, &run_id).await;
        return;
    }
    let response = handle(&state, req).await;
    // `handle` returns empty only for the SSE route, which is handled above.
    if !response.is_empty() {
        let _ = writer_half.write_all(&response).await;
    }
}

/// Bind and serve forever. One task per connection; the scheduler tick runs
/// beside this in `serve.rs`, not in here.
pub async fn serve(addr: SocketAddr, state: ServeState) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // A shared cap bounds tasks, sockets and (because SSE holds a permit for
    // its whole life) long-lived streams. Excess connections are dropped rather
    // than queued: a flood must not grow the accept backlog without limit.
    let connections = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    tracing::info!(%addr, "mixd serve: listening");
    loop {
        let (socket, peer) = listener.accept().await?;
        tracing::debug!(%peer, "serve: connection");
        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
            tracing::warn!(%peer, "serve: connection limit reached; dropping");
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit; // released on drop, including SSE timeouts
            serve_connection(state, socket).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_only_strips_query_and_fragment() {
        assert_eq!(path_only("/api/v1/runs?limit=5"), "/api/v1/runs");
        assert_eq!(
            path_only("/api/v1/runs/abc/events?token=x#frag"),
            "/api/v1/runs/abc/events"
        );
        assert_eq!(path_only("/health"), "/health");
    }

    #[test]
    fn query_param_extracts_values() {
        assert_eq!(
            query_param("/api/v1/runs?limit=5&token=abc", "limit").as_deref(),
            Some("5")
        );
        assert_eq!(
            query_param("/api/v1/runs/1/events?token=abc", "token").as_deref(),
            Some("abc")
        );
        assert!(query_param("/api/v1/runs", "token").is_none());
    }

    #[test]
    fn a_token_is_always_required_even_without_headers() {
        let none: &[(&str, &str)] = &[];
        assert!(!bearer_allowed("", none));
        assert!(!bearer_allowed("secret", none));
        assert!(bearer_allowed(
            "secret",
            &[("authorization", "Bearer secret")]
        ));
        assert!(!bearer_allowed(
            "secret",
            &[("authorization", "Bearer wrong")]
        ));
        assert!(bearer_allowed(
            "secret",
            &[("authorization", "bEaReR secret")]
        ));
    }

    #[test]
    fn host_and_origin_must_match_the_configured_target() {
        let origins = vec!["http://127.0.0.1:3031".to_string()];
        let hosts = vec!["127.0.0.1:3031".to_string()];
        assert!(target_allowed(
            &[
                ("host", "127.0.0.1:3031"),
                ("origin", "http://127.0.0.1:3031")
            ],
            &origins,
            &hosts
        ));
        // DNS rebinding: correct Host, hostile Origin (and vice versa).
        assert!(!target_allowed(
            &[
                ("host", "127.0.0.1:3031"),
                ("origin", "http://evil.example")
            ],
            &origins,
            &hosts
        ));
        assert!(!target_allowed(
            &[
                ("host", "evil.example"),
                ("origin", "http://127.0.0.1:3031")
            ],
            &origins,
            &hosts
        ));
        assert!(!target_allowed(
            &[("host", "127.0.0.1:3031"), ("origin", "null")],
            &origins,
            &hosts
        ));
        assert!(!target_allowed(&[], &origins, &hosts), "Host is required");
        // Non-browser clients send no Origin and rely on the bearer token.
        assert!(target_allowed(
            &[("host", "127.0.0.1:3031")],
            &origins,
            &hosts
        ));
        // A wildcard bind with no --origin rejects every browser origin.
        assert!(!target_allowed(&[("origin", "http://x")], &[], &[]));
        assert!(target_allowed(&[], &[], &[]));
    }

    #[test]
    fn decision_fields_accept_both_snake_and_camel_case() {
        let snake = serde_json::json!({
            "grant": true, "remember_session": true, "acknowledge_risk": true
        });
        let camel = serde_json::json!({
            "grant": true, "rememberSession": true, "acknowledgeRisk": true
        });
        for input in [&snake, &camel] {
            assert!(bool_alias(input, "grant", "grant"));
            assert!(bool_alias(input, "remember_session", "rememberSession"));
            assert!(bool_alias(input, "acknowledge_risk", "acknowledgeRisk"));
        }
        assert!(!bool_alias(&serde_json::json!({}), "grant", "grant"));
    }

    #[test]
    fn query_tokens_are_redacted_before_logging() {
        let redacted = redact_path("/api/v1/runs/1/events?token=super-secret&limit=5");
        assert!(!redacted.contains("super-secret"), "{redacted}");
        assert!(redacted.contains("<redacted>"), "{redacted}");
        assert!(redacted.contains("limit=5"), "{redacted}");
        assert_eq!(redact_path("/health"), "/health");
    }
}
