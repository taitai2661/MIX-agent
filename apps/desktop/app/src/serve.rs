//! `mix-agent-desktop serve` — headless HTTP bridge for the phone PWA.
//!
//! Same idea as `headless.rs` (`mix-agent run`): the engine, storage, tools
//! and providers are plain Rust, so they run without a window, tray or
//! WebView. Where `run` streams one Run's events to stdout, `serve` exposes
//! the approval inbox over HTTP (`http/mod.rs`) so a phone PWA can approve
//! tool calls while the process runs on a NAS.
//!
//! Deliberate skeleton limits:
//!
//! * No run dispatch yet: `commands::conversations::dispatch_run` and
//!   `scheduler::tick` take the full Tauri `AppState` (browser, MCP host,
//!   local runtimes, …). Decoupling that is the next step; until then this
//!   serves runs/approvals/schedules that already exist and streams live
//!   engine events.
//! * Approval policy is the **normal prompt** (`headless_approval` stays
//!   `None`): a tool call parks in `waiting_approval` so the phone can resolve
//!   it via `POST /api/v1/approvals/{id}/decision`. Setting `Some(false)`
//!   here (like `run` without `--yes`) would deny before the phone ever sees
//!   the prompt.

use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::state;

#[derive(Default)]
struct Args {
    bind: Option<String>,
    port: Option<u16>,
    token: Option<String>,
    origin: Option<String>,
    help: bool,
}

/// Where the persistent, process-table-free bearer token lives.
fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("serve.token")
}

/// Where the single-instance marker lives. Shared with `state.rs` so the GUI
/// can avoid reconciling Runs that a live `serve` process owns.
fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("serve.lock")
}

fn generate_token() -> String {
    // `Uuid::new_v4` draws from the OS CSPRNG (uuid's `v4` feature). Two of
    // them give 64 hex characters — far more than a bearer token needs.
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenSource {
    Env,
    File,
    Cli,
    Generated,
}

impl TokenSource {
    fn label(self) -> &'static str {
        match self {
            Self::Env => "MIX_SERVE_TOKEN",
            Self::File => "token file",
            Self::Cli => "--token (INSECURE)",
            Self::Generated => "newly generated",
        }
    }
}

/// A token file must be a private, regular file before its contents are used.
#[cfg(unix)]
fn token_file_permissions_ok(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file() && meta.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn token_file_permissions_ok(meta: &std::fs::Metadata) -> bool {
    meta.is_file()
}

/// Generate a token file atomically: `create_new` never follows a symlink and
/// never truncates a file another process may be reading. `0o600` is applied
/// at creation so there is no world-readable window.
fn create_token_file(path: &Path, token: &str) -> std::io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    file.sync_all().ok();
    Ok(file)
}

/// Resolve the bearer token, preferring the environment and the token file
/// over a CLI argument. A CLI token is visible to every local user via `ps`,
/// shell history and crash dumps, so it is a last resort with a loud warning.
fn resolve_token(data_dir: &Path, cli: Option<&str>) -> anyhow::Result<(String, TokenSource)> {
    if let Ok(value) = std::env::var("MIX_SERVE_TOKEN") {
        if !value.trim().is_empty() {
            return Ok((value, TokenSource::Env));
        }
    }
    let path = token_path(data_dir);
    // A token file is a secret: refuse to trust a symlink (an attacker with
    // write access to the data dir could point it at a world-readable file)
    // and on Unix refuse a group/world-readable regular file.
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.file_type().is_symlink() && token_file_permissions_ok(&meta) {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let value = raw.trim();
                if !value.is_empty() {
                    return Ok((value.to_string(), TokenSource::File));
                }
            }
        } else {
            eprintln!(
                "WARNING: ignoring {} (symlink or overly permissive mode); generating a fresh token",
                path.display()
            );
        }
    }
    if let Some(value) = cli.filter(|value| !value.trim().is_empty()) {
        eprintln!(
            "WARNING: --token exposes the bearer token in the process table (ps), \
             shell history and crash reports. Prefer MIX_SERVE_TOKEN or {}.",
            path.display()
        );
        return Ok((value.to_string(), TokenSource::Cli));
    }
    // Generate with `create_new`: a pre-existing file (including a symlink) is
    // never truncated, and the token is never written into a file another
    // process can read. If a concurrent starter won the race, fall back to its
    // token file rather than overwriting it.
    let token = generate_token();
    match create_token_file(&path, &token) {
        Ok(_file) => Ok((token, TokenSource::Generated)),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                let value = raw.trim();
                if !value.is_empty() {
                    return Ok((value.to_string(), TokenSource::File));
                }
            }
            Err(anyhow::anyhow!(
                "cannot write {}: token file exists but is unreadable",
                path.display()
            ))
        }
        Err(err) => Err(anyhow::anyhow!("cannot write {}: {err}", path.display())),
    }
}

/// Normalise `--origin` to a bare `scheme://host[:port]` string.
fn normalize_origin(raw: &str) -> anyhow::Result<String> {
    let url =
        url::Url::parse(raw.trim()).map_err(|err| anyhow::anyhow!("invalid --origin: {err}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
    {
        anyhow::bail!("--origin must be a bare http(s)://host[:port] origin");
    }
    Ok(url.origin().ascii_serialization())
}

fn authority_from_origin(origin: &str) -> String {
    origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .trim_end_matches('/')
        .to_string()
}

fn authority_for(ip: IpAddr, port: u16) -> String {
    if ip.is_ipv6() {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    }
}

/// Host/Origin values a browser is allowed to present.
///
/// With an explicit `--origin` it is authoritative; otherwise we derive it
/// from the bound address and add loopback aliases so `localhost` and `127.0.0.1`
/// in a browser discover the same server. A wildcard bind without `--origin`
/// yields an empty origin list: non-browser clients (no `Origin` header) still
/// work with the bearer token, while every browser Origin is rejected. That is
/// intentional fail-closed behaviour for DNS-rebinding protection.
fn allowed_request_targets(
    bind_ip: IpAddr,
    port: u16,
    explicit: Option<String>,
) -> (Vec<String>, Vec<String>) {
    let mut origins = Vec::new();
    let mut hosts = Vec::new();
    if let Some(origin) = explicit {
        origins.push(origin.clone());
        hosts.push(authority_from_origin(&origin));
    } else if !bind_ip.is_unspecified() {
        let authority = authority_for(bind_ip, port);
        origins.push(format!("http://{authority}"));
        hosts.push(authority);
    }
    if bind_ip.is_loopback() {
        // A PWA served from any of these spellings must reach the same API.
        for authority in [
            format!("127.0.0.1:{port}"),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
        ] {
            if !hosts.contains(&authority) {
                origins.push(format!("http://{authority}"));
                hosts.push(authority);
            }
        }
    }
    (origins, hosts)
}

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Is the process that wrote `path` still alive?
///
/// On Unix this is a real `kill(pid, 0)` liveness probe (EPERM means alive but
/// owned by another user). On Windows it is a real `OpenProcess` +
/// `WaitForSingleObject(0)` probe: an access-denied result is treated as alive,
/// because the process exists but belongs to a different user. A recycled PID
/// can still look alive on either platform; a per-run generation tag in the
/// database would remove that caveat but needs a storage schema change.
fn lock_owner_alive(pid: i32, started: i64) -> bool {
    // Each platform uses one of the two values; silence the other's warning.
    let _ = (pid, started);
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only probes for existence; no signal is delivered.
        unsafe {
            libc::kill(pid, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{
            CloseHandle, GetLastError, ERROR_ACCESS_DENIED, WAIT_OBJECT_0,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };
        if handle == 0 {
            // ERROR_ACCESS_DENIED means the PID exists but is owned by another
            // user: fail closed and treat the lock as live.
            return unsafe { GetLastError() == ERROR_ACCESS_DENIED };
        }
        let signaled = unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 };
        unsafe { CloseHandle(handle) };
        !signaled
    }
    #[cfg(not(any(unix, windows)))]
    {
        // No process-table API: keep the conservative freshness window.
        unix_now().saturating_sub(started) < 600
    }
}

fn lock_owner(path: &Path) -> Option<(i32, i64)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut lines = raw.lines();
    let pid = lines.next()?.parse::<i32>().ok()?;
    // line 2 is the generation (opaque); line 3 is the start timestamp.
    let started = lines.nth(1)?.parse::<i64>().ok()?;
    Some((pid, started))
}

fn lock_is_stale(path: &Path) -> bool {
    match lock_owner(path) {
        Some((pid, started)) => !lock_owner_alive(pid, started),
        None => true, // corrupt marker: reclaim it
    }
}

/// True when a live `serve` process currently owns this data directory.
///
/// `build_app_state` uses this to skip its crash reconciliation: interrupting
/// Runs owned by a running server would corrupt work the phone is driving.
pub(crate) fn serve_instance_active(data_dir: &Path) -> bool {
    match lock_owner(&lock_path(data_dir)) {
        Some((pid, started)) => lock_owner_alive(pid, started),
        None => false,
    }
}

/// Exclusive marker for one `serve` process per data directory.
///
/// Holding the `File` only reserves the path; the generation string in the
/// file lets `Drop` avoid deleting a lock a later process reclaimed after this
/// one died without running its destructor (SIGKILL).
struct ServeInstanceLock {
    path: PathBuf,
    generation: String,
}

impl ServeInstanceLock {
    fn acquire(data_dir: &Path) -> anyhow::Result<Self> {
        let path = lock_path(data_dir);
        for _ in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let generation = uuid::Uuid::new_v4().to_string();
                    let contents =
                        format!("{}\n{}\n{}\n", std::process::id(), generation, unix_now());
                    file.write_all(contents.as_bytes())?;
                    file.sync_all().ok();
                    return Ok(Self { path, generation });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !lock_is_stale(&path) {
                        anyhow::bail!(
                            "another `mix-agent serve` owns {}; stop it (or remove the stale \
                             lock file) before starting a second instance",
                            path.display()
                        );
                    }
                    let _ = std::fs::remove_file(&path);
                }
                Err(err) => return Err(err.into()),
            }
        }
        anyhow::bail!("could not acquire the serve instance lock")
    }
}

impl Drop for ServeInstanceLock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).is_ok_and(|raw| raw.contains(&self.generation)) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn serve_if_requested() -> Option<i32> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) != Some("serve") {
        return None;
    }
    let args = match parse_args(&argv[1..]) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("mix-agent serve: {err}\n\n{}", serve_help());
            return Some(2);
        }
    };
    if args.help {
        println!("{}", serve_help());
        return Some(0);
    }
    Some(run(args))
}

fn serve_help() -> &'static str {
    "USAGE:\n    mix-agent-desktop serve [--bind ADDR] [--port PORT] [--origin URL] [--token TOKEN]\n\n\
     OPTIONS:\n\
     \x20       --bind <ADDR>   Listen address (default: 127.0.0.1)\n\
     \x20       --port <PORT>   Listen port (default: 3031)\n\
     \x20       --origin <URL>  Browser origin allowed to call /api/v1 (default: from --bind)\n\
     \x20       --token <TOKEN> DISCOURAGED: visible in the process table; prefer\n\
     \x20                       $MIX_SERVE_TOKEN or <data-dir>/serve.token\n\
     \x20   -h, --help          Show this help\n\n\
     A bearer token is always required on /api/v1, even on loopback. Without an\n\
     explicit token one is generated, saved in <data-dir>/serve.token (0600) and\n\
     printed at startup. Host/Origin headers are checked against --origin to stop\n\
     browser DNS rebinding. A non-loopback --bind still refuses to start without\n\
     a token (fail-closed).\n\
     Shares the desktop database ($MIX_DATA_DIR) so the GUI and serve see the same runs."
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut index = 0;
    while index < argv.len() {
        let value = argv[index].as_str();
        match value {
            "--bind" => args.bind = Some(take_value(argv, &mut index, value)?),
            "--port" => {
                let raw = take_value(argv, &mut index, value)?;
                args.port = Some(
                    raw.parse()
                        .map_err(|_| format!("--port must be a number: {raw}"))?,
                );
            }
            "--token" => args.token = Some(take_value(argv, &mut index, value)?),
            "--origin" => args.origin = Some(take_value(argv, &mut index, value)?),
            "-h" | "--help" => args.help = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}"));
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
        index += 1;
    }
    Ok(args)
}

fn take_value(argv: &[String], index: &mut usize, name: &str) -> Result<String, String> {
    *index += 1;
    argv.get(*index)
        .cloned()
        .ok_or_else(|| format!("{name} requires a value"))
}

fn run(args: Args) -> i32 {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    crate::startup::init_tracing();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("mix-agent serve: could not start the async runtime: {err}");
            return 1;
        }
    };
    match runtime.block_on(execute(args)) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("mix-agent serve: {err:#}");
            1
        }
    }
}

async fn execute(args: Args) -> anyhow::Result<()> {
    let bind = args.bind.as_deref().unwrap_or("127.0.0.1").to_string();
    let port = args.port.unwrap_or(3031);
    let addr: SocketAddr = format!("{bind}:{port}")
        .parse()
        .map_err(|err| anyhow::anyhow!("invalid --bind/--port: {err}"))?;
    let data_dir = crate::cli::data_dir();
    std::fs::create_dir_all(&data_dir)?;
    // One server per data directory: two `serve` processes would both own the
    // same approvals/engine state and could resolve the same prompt twice.
    // Held for the lifetime of `execute`.
    let _instance = ServeInstanceLock::acquire(&data_dir)?;

    // A bearer token is always present now: env var, token file, a discouraged
    // CLI argument, or a freshly generated random one. The fail-closed
    // non-loopback check is therefore a defensive assertion, not the policy.
    let (token, source) = resolve_token(&data_dir, args.token.as_deref())?;
    if token.is_empty() && !addr.ip().is_loopback() {
        anyhow::bail!("refusing to listen on non-loopback {addr} without a token");
    }
    let explicit_origin = args.origin.as_deref().map(normalize_origin).transpose()?;
    let (allowed_origins, allowed_hosts) =
        allowed_request_targets(addr.ip(), port, explicit_origin);

    let storage = Arc::new(mixd_storage::Storage::open(&data_dir).await?);
    let secrets = Arc::new(mixd_storage::SecretStore::open(&data_dir));
    let settings = crate::commands::settings::read_settings(storage.pool())
        .await
        .unwrap_or_default();
    let egress = state::load_egress_policy(storage.as_ref()).await;

    let mcp = Arc::new(crate::McpHost::new());
    let agent_runner = Arc::new(crate::EngineAgentRunner::default());
    let embeddings = Arc::new(crate::embedding::EmbeddingResolver::new(
        storage.clone(),
        secrets.clone(),
    ));
    let tools = state::build_registry(
        &storage,
        None,
        &mcp,
        &agent_runner,
        &embeddings,
        egress,
        settings.discover_user_skills,
    );
    let engine = Arc::new(mixd_core::runs::engine::RunEngine::with_storage_and_tools(
        storage.clone(),
        secrets.clone(),
        tools,
    ));
    agent_runner.attach(&engine, &secrets, "");
    state::register_providers_from_storage(&engine.providers, &storage, &secrets).await?;
    let sandbox = settings.sandbox_terminal && mixd_tools::terminal::os_sandbox_available();
    engine.set_terminal_sandbox(sandbox).await;
    // Leave `headless_approval` unset on purpose: tool calls must park in
    // `waiting_approval` so the phone PWA can resolve them. See module docs.

    let state = crate::http::ServeState {
        storage,
        engine,
        token: token.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        allowed_origins,
        allowed_hosts,
    };
    eprintln!("mix-agent serve: listening on http://{addr} (health: /health)");
    // Print the secret once at startup so a generated token is usable. The
    // server itself never logs a raw `?token=` (see `http::redact_path`).
    eprintln!(
        "mix-agent serve: bearer token (source: {}): {token}",
        source.label()
    );
    if source == TokenSource::Generated || source == TokenSource::File {
        eprintln!(
            "mix-agent serve: token file: {}",
            token_path(&data_dir).display()
        );
    }
    crate::http::serve(addr, state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_bind_port_and_token() {
        let args = parse_args(&argv(&[
            "--bind", "0.0.0.0", "--port", "3031", "--token", "secret",
        ]))
        .expect("parse");
        assert_eq!(args.bind.as_deref(), Some("0.0.0.0"));
        assert_eq!(args.port, Some(3031));
        assert_eq!(args.token.as_deref(), Some("secret"));
    }

    #[test]
    fn rejects_an_unknown_flag() {
        assert!(parse_args(&argv(&["--nope"])).is_err());
    }

    #[test]
    fn port_must_be_a_number() {
        assert!(parse_args(&argv(&["--port", "lots"])).is_err());
    }

    #[test]
    fn parses_the_origin_flag() {
        let args = parse_args(&argv(&[
            "--origin",
            "http://192.168.1.9:3031",
            "--port",
            "3032",
        ]))
        .expect("parse");
        assert_eq!(args.origin.as_deref(), Some("http://192.168.1.9:3031"));
        assert_eq!(args.port, Some(3032));
    }

    #[test]
    fn origins_must_be_bare_http_origins() {
        assert_eq!(
            normalize_origin("http://192.168.1.9:3031/").unwrap(),
            "http://192.168.1.9:3031"
        );
        assert!(normalize_origin("file:///tmp/x").is_err());
        assert!(normalize_origin("http://host:3031/api").is_err());
    }

    #[test]
    fn a_loopback_bind_trusts_all_loopback_spellings() {
        let (origins, hosts) = allowed_request_targets("127.0.0.1".parse().unwrap(), 3031, None);
        for host in ["127.0.0.1:3031", "localhost:3031", "[::1]:3031"] {
            assert!(hosts.iter().any(|h| h == host), "missing host {host}");
            assert!(
                origins.iter().any(|o| o == &format!("http://{host}")),
                "missing origin for {host}"
            );
        }
        // A wildcard bind cannot know the Host header; browsers must send a
        // matching --origin and non-browser clients rely on the token.
        let (origins, hosts) = allowed_request_targets("0.0.0.0".parse().unwrap(), 3031, None);
        assert!(origins.is_empty() && hosts.is_empty());
    }

    #[test]
    fn a_stale_lock_is_detected_by_dead_pid() {
        let dir =
            std::env::temp_dir().join(format!("mix-serve-lock-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = lock_path(&dir);
        // 99_999_999 is far above any real pid; the timestamp is irrelevant on
        // Unix, and on Windows this is old enough to be stale too.
        std::fs::write(&path, format!("99999999\n{}\n0\n", uuid::Uuid::new_v4())).unwrap();
        assert!(lock_is_stale(&path));
        assert!(!serve_instance_active(&dir));
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n",
                std::process::id(),
                uuid::Uuid::new_v4(),
                unix_now()
            ),
        )
        .unwrap();
        assert!(!lock_is_stale(&path));
        assert!(serve_instance_active(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn generated_tokens_are_long_hex_strings() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token, generate_token());
    }
}
