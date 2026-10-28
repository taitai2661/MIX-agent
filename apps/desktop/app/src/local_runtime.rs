//! Local inference runtime lifecycle and model operations.
//!
//! Discovery (`mixd_providers::discovery`) answers what exists. This module
//! owns the runtime's *state*: whether the app started it, its process, its log
//! tail, health transitions, and the long-running pull/load jobs. Everything
//! here is reachable only through `#[tauri::command]` in
//! `commands::local_runtime`; the WebView has no process or HTTP capability.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use futures::future::join_all;
use mixd_providers::discovery::{
    self, DetectedRuntime, DiscoveredModel, RuntimeId, RuntimeLaunchSpec, RuntimeStop, ScanReport,
};
use mixd_providers::local_error::{error, simple};
use mixd_providers::local_ops::{self, CancelToken, PullProgress};
use mixd_storage::Storage;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{Mutex, RwLock};

use crate::commands::providers::is_managed_provider;
use crate::commands::settings::{read_settings, SettingsView};

const LOG_LIMIT: usize = 500;
const HEALTH_INTERVAL: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Unknown,
    NotInstalled,
    Stopped,
    Starting,
    Online,
    Degraded,
    Stopping,
    Error,
}

impl RuntimeState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::NotInstalled => "not_installed",
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Online => "online",
            Self::Degraded => "degraded",
            Self::Stopping => "stopping",
            Self::Error => "error",
        }
    }
}

/// Which model operations a runtime supports.
///
/// Kept in one place so the manager snapshot and the discovery table cannot
/// disagree, and so the UI never has to branch on a vendor slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelOperations {
    pub pull: bool,
    pub delete: bool,
    pub load: bool,
    pub unload: bool,
}

pub fn model_operations(slug: &str) -> ModelOperations {
    match slug {
        "ollama" => ModelOperations {
            pull: true,
            delete: true,
            load: true,
            unload: true,
        },
        "lmstudio" => {
            let cli = lms_available();
            ModelOperations {
                pull: cli,
                // LM Studio has no stable CLI delete across versions.
                delete: false,
                load: cli,
                unload: cli,
            }
        }
        _ => ModelOperations {
            pull: false,
            delete: false,
            load: false,
            unload: false,
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSnapshot {
    pub slug: String,
    pub label: String,
    pub kind: String,
    pub state: String,
    pub online: bool,
    pub installed: bool,
    pub managed: bool,
    pub endpoint: String,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub install_hint: Option<String>,
    pub launcher: Option<String>,
    pub error: Option<String>,
    pub models: Vec<DiscoveredModel>,
    /// Capability booleans are computed from the runtime, not hardcoded in the
    /// UI, so a runtime without a CLI never renders a dead button.
    pub can_start: bool,
    pub can_stop: bool,
    pub can_open_app: bool,
    pub can_pull: bool,
    pub can_delete: bool,
    pub can_load: bool,
    pub can_unload: bool,
    /// True after the user explicitly stopped this runtime. Send-time
    /// auto-start must not undo that decision; an explicit Start clears it.
    pub auto_start_suppressed: bool,
}

#[derive(Clone)]
struct RuntimeRecord {
    slug: String,
    label: String,
    kind: String,
    state: RuntimeState,
    endpoint: String,
    version: Option<String>,
    installed: bool,
    install_hint: Option<String>,
    executable: Option<String>,
    launcher: Option<String>,
    managed: bool,
    error: Option<String>,
    failures: u8,
    models: Vec<DiscoveredModel>,
    /// Cached from `launch_spec` / `open_app_target` at record creation and
    /// on each scan: `snapshot()` is polled every 10 s and must not walk PATH
    /// and the install roots on every call.
    can_start: bool,
    can_open_app: bool,
    auto_start_suppressed: bool,
    logs: Arc<StdMutex<VecDeque<String>>>,
}

impl RuntimeRecord {
    fn new(runtime: RuntimeId) -> Self {
        let spec = discovery::launch_spec(runtime);
        let open_app = discovery::open_app_target(runtime);
        Self {
            slug: runtime.slug().to_string(),
            label: runtime.label().to_string(),
            kind: runtime.kind().as_str().to_string(),
            state: RuntimeState::Unknown,
            endpoint: discovery::default_base_url(runtime),
            version: None,
            installed: spec.is_some() || open_app.is_some(),
            install_hint: open_app.as_ref().map(|path| path.display().to_string()),
            executable: spec.as_ref().map(|spec| spec.program.display().to_string()),
            launcher: None,
            managed: false,
            error: None,
            failures: 0,
            models: Vec::new(),
            can_start: spec.is_some(),
            can_open_app: open_app.is_some(),
            auto_start_suppressed: false,
            logs: Arc::new(StdMutex::new(VecDeque::new())),
        }
    }

    fn online(&self) -> bool {
        matches!(self.state, RuntimeState::Online | RuntimeState::Degraded)
    }

    fn snapshot(&self) -> RuntimeSnapshot {
        let operations = model_operations(&self.slug);
        RuntimeSnapshot {
            slug: self.slug.clone(),
            label: self.label.clone(),
            kind: self.kind.clone(),
            state: self.state.as_str().to_string(),
            online: self.online(),
            installed: self.installed,
            managed: self.managed,
            endpoint: self.endpoint.clone(),
            version: self.version.clone(),
            executable: self.executable.clone(),
            install_hint: self.install_hint.clone(),
            launcher: self.launcher.clone(),
            error: self.error.clone(),
            models: self.models.clone(),
            can_start: self.can_start,
            can_stop: self.managed,
            can_open_app: self.can_open_app,
            can_pull: operations.pull,
            can_delete: operations.delete,
            can_load: operations.load,
            can_unload: operations.unload,
            auto_start_suppressed: self.auto_start_suppressed,
        }
    }
}

/// Whether the LM Studio CLI is installed. `snapshot()` used to call
/// `find_on_path("lms")` for every runtime on every poll (10 s), each a full
/// PATH scan; the answer cannot change while the process runs.
fn lms_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| discovery::find_on_path("lms").is_some())
}

struct ChildHandle {
    child: tokio::process::Child,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    pub id: String,
    pub runtime: String,
    pub kind: String,
    pub model: String,
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub message: String,
    pub error: Option<String>,
}

/// `(id, kind, preset, baseUrl)` for an enabled provider row.
type ProviderEndpointRow = (String, Option<String>, Option<String>, Option<String>);
/// `(autoStart, kind, preset, baseUrl)` for an enabled provider row.
type ProviderAutoStartRow = (Option<i64>, Option<String>, Option<String>, Option<String>);

struct Job {
    view: JobView,
    cancel: CancelToken,
    started: Instant,
    /// When the job reached a terminal status, for pruning.
    finished: Option<Instant>,
}

/// Terminal jobs are kept for a while so the panel can show "complete", then
/// dropped: a long-lived app used to accumulate every pull it ever ran.
const JOB_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_JOBS: usize = 100;

fn prune_jobs(jobs: &mut HashMap<String, Job>, now: Instant) {
    jobs.retain(|_, job| {
        job.finished
            .is_none_or(|at| now.saturating_duration_since(at) < JOB_TTL)
    });
    if jobs.len() <= MAX_JOBS {
        return;
    }
    let mut finished: Vec<(String, Instant)> = jobs
        .iter()
        .filter_map(|(id, job)| job.finished.map(|at| (id.clone(), at)))
        .collect();
    finished.sort_by_key(|(_, at)| *at);
    for (id, _) in finished.into_iter().take(jobs.len() - MAX_JOBS) {
        jobs.remove(&id);
    }
}

pub struct LocalRuntimeManager {
    app: AppHandle,
    storage: Arc<Storage>,
    records: RwLock<HashMap<String, RuntimeRecord>>,
    children: Mutex<HashMap<String, ChildHandle>>,
    /// One lock per slug, so start/stop/restart cannot interleave and leave a
    /// managed process the records no longer know about.
    slug_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    jobs: StdMutex<HashMap<String, Job>>,
    initialized: AtomicBool,
    init_lock: Mutex<()>,
    health_started: AtomicBool,
    health_ticks: AtomicU64,
}

impl LocalRuntimeManager {
    pub fn new(app: AppHandle, storage: Arc<Storage>) -> Arc<Self> {
        Arc::new(Self {
            app,
            storage,
            records: RwLock::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
            slug_locks: Mutex::new(HashMap::new()),
            jobs: StdMutex::new(HashMap::new()),
            initialized: AtomicBool::new(false),
            init_lock: Mutex::new(()),
            health_started: AtomicBool::new(false),
            health_ticks: AtomicU64::new(0),
        })
    }

    async fn slug_lock(&self, slug: &str) -> Arc<Mutex<()>> {
        self.slug_locks
            .lock()
            .await
            .entry(slug.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub async fn snapshot(&self) -> Vec<RuntimeSnapshot> {
        self.ensure_loaded().await;
        let records = self.records.read().await;
        let mut snapshots: Vec<RuntimeSnapshot> =
            records.values().map(RuntimeRecord::snapshot).collect();
        snapshots.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
        });
        snapshots
    }

    async fn ensure_loaded(&self) {
        if self.initialized.load(Ordering::Relaxed) {
            return;
        }
        let _guard = self.init_lock.lock().await;
        if self.initialized.load(Ordering::Relaxed) {
            return;
        }
        // The WebView can ask for a snapshot before `startup()` has run. Honour
        // the opt-out here too instead of sweeping the loopback anyway.
        let scan_on_start = read_settings(self.storage.pool())
            .await
            .map(|settings| settings.local_scan_on_start)
            .unwrap_or(true);
        if !scan_on_start {
            self.initialized.store(true, Ordering::Relaxed);
            return;
        }
        if self.load_once().await {
            self.initialized.store(true, Ordering::Relaxed);
        }
    }

    /// Run the fast sweep and ingest it. Returns whether the scan succeeded;
    /// unlike the old `swap(true)` before the scan, a transient failure no
    /// longer leaves the runtime table permanently empty.
    async fn load_once(&self) -> bool {
        match discovery::scan_local_runtimes().await {
            Ok(report) => {
                self.ingest_scan(&report).await;
                true
            }
            Err(err) => {
                tracing::warn!(%err, "local runtime scan failed");
                false
            }
        }
    }

    pub async fn ingest_scan(&self, report: &ScanReport) {
        let mut records = self.records.write().await;
        for runtime in &report.runtimes {
            let Some(id) = RuntimeId::from_slug(&runtime.slug) else {
                continue;
            };
            let spec = discovery::launch_spec(id);
            let open_app = discovery::open_app_target(id);
            let entry = records
                .entry(runtime.slug.clone())
                .or_insert_with(|| RuntimeRecord::new(id));
            entry.label = runtime.label.clone();
            entry.kind = runtime.kind.clone();
            entry.endpoint = if runtime.online {
                runtime.base_url.clone()
            } else {
                discovery::default_base_url(id)
            };
            entry.version = runtime.version.clone();
            entry.installed = runtime.installed || spec.is_some() || open_app.is_some();
            entry.can_start = spec.is_some();
            entry.can_open_app = open_app.is_some();
            entry.install_hint = runtime
                .install_hint
                .clone()
                .or_else(|| open_app.map(|path| path.display().to_string()));
            entry.executable = spec
                .map(|spec| spec.program.display().to_string())
                .or(entry.executable.clone());
            entry.launcher = runtime.launcher.clone();
            // A scan must never overwrite an in-flight transition. It is a
            // snapshot of the outside world, not of our process lifecycle.
            if !matches!(entry.state, RuntimeState::Starting | RuntimeState::Stopping) {
                entry.state = if runtime.online {
                    RuntimeState::Online
                } else if runtime.listening {
                    RuntimeState::Error
                } else if entry.installed {
                    RuntimeState::Stopped
                } else {
                    RuntimeState::NotInstalled
                };
            }
            entry.error = runtime.error.clone();
            if runtime.online {
                entry.models = runtime.models.clone();
                entry.failures = 0;
            }
        }
    }

    pub async fn refresh(&self) -> anyhow::Result<Vec<RuntimeSnapshot>> {
        let report = discovery::scan_local_runtimes_detailed().await?;
        self.ingest_scan(&report).await;
        self.initialized.store(true, Ordering::Relaxed);
        Ok(self.snapshot().await)
    }

    pub async fn start(&self, slug: &str) -> anyhow::Result<RuntimeSnapshot> {
        let lock = self.slug_lock(slug).await;
        let _guard = lock.lock().await;
        self.start_locked(slug).await
    }

    async fn start_locked(&self, slug: &str) -> anyhow::Result<RuntimeSnapshot> {
        let runtime = RuntimeId::from_slug(slug)
            .ok_or_else(|| error("start_unsupported", &[slug.to_string()]))?;
        let spec = discovery::launch_spec(runtime)
            .ok_or_else(|| error("launcher_missing", &[runtime.label().to_string()]))?;
        // Prefer the provider's configured base URL. A user running Ollama on
        // 11500 (or at a LAN address) must not get a second server spawned on
        // the 11434 default just because the manager started from a scan row.
        let endpoint = self.endpoint_for(slug).await;

        // If the runtime is already serving, never claim ownership. The record
        // can be stale (the user started it from a terminal) and a stop on app
        // exit must not kill something we did not launch.
        if let Ok(found) = discovery::probe_base_url(&endpoint).await {
            if found.online {
                let models = detailed_models(&endpoint, found).await;
                {
                    let mut records = self.records.write().await;
                    let record = records
                        .entry(slug.to_string())
                        .or_insert_with(|| RuntimeRecord::new(runtime));
                    record.state = RuntimeState::Online;
                    record.managed = false;
                    record.endpoint = endpoint;
                    record.models = models;
                    record.error = None;
                    record.failures = 0;
                }
                self.emit_runtime(slug).await;
                return self.one(slug).await;
            }
        }

        {
            let mut records = self.records.write().await;
            let record = records
                .entry(slug.to_string())
                .or_insert_with(|| RuntimeRecord::new(runtime));
            record.state = RuntimeState::Starting;
            record.managed = true;
            record.error = None;
            record.failures = 0;
            // An explicit start (or a successful auto-start) cancels the
            // "user stopped this" suppression.
            record.auto_start_suppressed = false;
            record.endpoint = endpoint.clone();
            // A fresh run should not show the previous process's tail.
            let logs = record.logs.clone();
            if let Ok(mut logs) = logs.lock() {
                logs.clear();
            };
        }
        self.emit_runtime(slug).await;

        let spec = retarget_spec(spec, runtime, &endpoint);
        let launch = if spec.stop == RuntimeStop::Child {
            self.spawn_child(slug, &spec).await
        } else {
            run_cli(&spec).await
        };
        let mut started_by_app = true;
        if let Err(err) = launch {
            // A server may already be running while our record was stale (the
            // user started it from a terminal, or an earlier stop failed).
            // Treat an answering endpoint as success, but do not claim
            // ownership: stop must not kill a process we did not start.
            let already_online = discovery::probe_base_url(&endpoint)
                .await
                .is_ok_and(|found| found.online);
            if already_online {
                started_by_app = false;
            } else {
                self.set_state(slug, RuntimeState::Error, Some(format!("{err:#}")))
                    .await;
                return Err(err);
            }
        }

        match wait_online(&endpoint, START_TIMEOUT).await {
            Ok(found) => {
                let models = detailed_models(&endpoint, found).await;
                let mut records = self.records.write().await;
                if let Some(record) = records.get_mut(slug) {
                    record.state = RuntimeState::Online;
                    record.managed = started_by_app;
                    record.endpoint = endpoint;
                    record.models = models;
                    record.error = None;
                    record.failures = 0;
                }
                drop(records);
                self.emit_runtime(slug).await;
                self.one(slug).await
            }
            Err(err) => {
                // Only clean up what this call launched. If the endpoint was
                // already served by an external process, `lms server stop`
                // would take down a server the app does not own.
                if started_by_app {
                    let _ = self.stop_owned(slug, false).await;
                }
                let coded = error(
                    "unresponsive",
                    &[runtime.label().to_string(), format!("{err:#}")],
                );
                self.set_state(slug, RuntimeState::Error, Some(format!("{coded:#}")))
                    .await;
                Err(coded)
            }
        }
    }

    async fn spawn_child(&self, slug: &str, spec: &RuntimeLaunchSpec) -> anyhow::Result<()> {
        let logs = {
            let records = self.records.read().await;
            records.get(slug).map(|record| record.logs.clone())
        }
        .unwrap_or_else(|| Arc::new(StdMutex::new(VecDeque::new())));

        let mut command = tokio::process::Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Deliberately NOT `kill_on_drop`: dropping the manager at app
            // exit would then kill the runtime even when the user asked to
            // keep it alive (`local_stop_on_exit = false`). Shutdown stops
            // owned children explicitly instead.
            .kill_on_drop(false);
        if slug == "ollama" {
            // Make the UI's keep_alive setting the server default for every
            // request, including the first one before the user explicitly
            // loads a model.
            if let Some(keep_alive) = read_settings(self.storage.pool())
                .await
                .ok()
                .and_then(|settings| settings.local_keep_alive)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                command.env("OLLAMA_KEEP_ALIVE", keep_alive);
            }
        }
        let mut child = command.spawn().map_err(|err| {
            error(
                "spawn_failed",
                &[spec.program.display().to_string(), err.to_string()],
            )
        })?;
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, logs.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, logs);
        }
        let previous = self
            .children
            .lock()
            .await
            .insert(slug.to_string(), ChildHandle { child });
        if let Some(previous) = previous {
            // Never leave the old process unmanaged: `kill_on_drop` is off (so
            // a clean quit can keep runtimes alive), which means a replaced
            // handle would otherwise be forgotten rather than stopped.
            tracing::warn!(slug, "stopping a runtime child that was still tracked");
            tokio::spawn(async move { terminate_child(previous).await });
        }
        Ok(())
    }

    pub async fn stop(&self, slug: &str) -> anyhow::Result<RuntimeSnapshot> {
        let lock = self.slug_lock(slug).await;
        let _guard = lock.lock().await;
        self.stop_locked(slug).await
    }

    async fn stop_locked(&self, slug: &str) -> anyhow::Result<RuntimeSnapshot> {
        let managed = self
            .records
            .read()
            .await
            .get(slug)
            .map(|record| record.managed)
            .unwrap_or(false);
        if !managed {
            return Err(simple("stop_external"));
        }
        self.stop_owned(slug, true).await?;
        self.one(slug).await
    }

    /// Stop without the external-process guard. Used by `start` cleanup and by
    /// app shutdown, where the record may already be in `Error`.
    ///
    /// `suppress_auto_start` is true only for a user-initiated stop: a failed
    /// start that cleans up its own child must not make the next send refuse to
    /// try again. App shutdown does not matter (the manager is going away), but
    /// it also passes false so the semantics stay obvious.
    async fn stop_owned(&self, slug: &str, suppress_auto_start: bool) -> anyhow::Result<()> {
        let managed = {
            let mut records = self.records.write().await;
            match records.get_mut(slug) {
                Some(record) => {
                    record.state = RuntimeState::Stopping;
                    record.managed
                }
                None => false,
            }
        };
        self.emit_runtime(slug).await;

        if let Some(child) = self.children.lock().await.remove(slug) {
            terminate_child(child).await;
        } else if managed {
            // Only ask the runtime's own CLI to stop a daemon this app started.
            if let Some(runtime) = RuntimeId::from_slug(slug) {
                if let Some(spec) = discovery::launch_spec(runtime) {
                    if let RuntimeStop::Cli { program, args } = spec.stop {
                        let output = tokio::process::Command::new(&program)
                            .args(&args)
                            .output()
                            .await;
                        if let Ok(output) = output {
                            if !output.status.success() {
                                tracing::warn!(
                                    slug,
                                    stderr = %String::from_utf8_lossy(&output.stderr),
                                    "runtime stop command failed"
                                );
                            }
                        }
                    }
                }
            }
        }

        let mut records = self.records.write().await;
        if let Some(record) = records.get_mut(slug) {
            record.state = RuntimeState::Stopped;
            record.managed = false;
            record.auto_start_suppressed = suppress_auto_start;
            record.error = None;
            record.failures = 0;
            for model in &mut record.models {
                model.loaded = false;
            }
        }
        drop(records);
        self.emit_runtime(slug).await;
        Ok(())
    }

    pub async fn restart(&self, slug: &str) -> anyhow::Result<RuntimeSnapshot> {
        let lock = self.slug_lock(slug).await;
        let _guard = lock.lock().await;
        // An external runtime is allowed to restart as "verify it is online":
        // the stop failure is expected, and start() will not claim ownership.
        let _ = self.stop_locked(slug).await;
        self.start_locked(slug).await
    }

    pub async fn logs(&self, slug: &str) -> Vec<String> {
        self.records
            .read()
            .await
            .get(slug)
            .and_then(|record| record.logs.lock().ok())
            .map(|logs| logs.iter().cloned().collect())
            .unwrap_or_default()
    }

    async fn one(&self, slug: &str) -> anyhow::Result<RuntimeSnapshot> {
        self.records
            .read()
            .await
            .get(slug)
            .map(RuntimeRecord::snapshot)
            .ok_or_else(|| error("unknown_runtime", &[slug.to_string()]))
    }

    async fn set_state(&self, slug: &str, state: RuntimeState, error: Option<String>) {
        {
            let mut records = self.records.write().await;
            if let Some(record) = records.get_mut(slug) {
                record.state = state;
                record.error = error;
                if !matches!(state, RuntimeState::Online | RuntimeState::Degraded) {
                    record.managed = false;
                }
            }
        }
        self.emit_runtime(slug).await;
    }

    async fn emit_runtime(&self, slug: &str) {
        let snapshot = self
            .records
            .read()
            .await
            .get(slug)
            .map(RuntimeRecord::snapshot);
        if let Some(snapshot) = snapshot {
            let _ = self.app.emit("local_runtime_event", snapshot);
        }
    }

    pub async fn refresh_models(&self, slug: &str) -> anyhow::Result<Vec<DiscoveredModel>> {
        let endpoint = self.endpoint_for(slug).await;
        let found = discovery::probe_base_url_detailed(&endpoint)
            .await
            .map_err(|err| error("probe_failed", &[endpoint.clone(), format!("{err:#}")]))?;
        {
            let mut records = self.records.write().await;
            if let Some(record) = records.get_mut(slug) {
                record.models = found.models.clone();
                record.version = found.version.clone();
                record.endpoint = found.base_url.clone();
                record.installed = record.installed || found.installed;
                record.failures = 0;
                record.error = found.error.clone();
                if found.online {
                    record.state = RuntimeState::Online;
                } else if record.state == RuntimeState::Online {
                    record.state = RuntimeState::Degraded;
                }
            }
        }
        self.emit_runtime(slug).await;
        // Keep the Composer's model table in step with a pull/delete/load. The
        // manager is not the only reader of `models`, so a runtime-only update
        // would leave the model picker offering a deleted tag.
        //
        // Only a reachable runtime is authoritative. A probe that found an
        // unrelated service on the port (or a refused connection) returns an
        // empty model list, and pruning from that would erase the whole table.
        if found.online {
            if let Some(provider_id) = self.provider_id_for_endpoint(slug, &found.base_url).await {
                match crate::commands::models::sync_models(
                    self.storage.pool(),
                    &provider_id,
                    &found.models,
                )
                .await
                {
                    Ok(_) => clear_missing_default_model(self.storage.pool()).await,
                    Err(err) => {
                        tracing::warn!(slug, provider_id, %err, "could not sync local models to the database");
                    }
                }
            }
        }
        Ok(found.models)
    }

    /// The enabled provider row for this runtime *at this endpoint*.
    ///
    /// Matching on kind/preset alone used to attach a local pull to whichever
    /// provider sorted first, which rewrote the model table of a remote Ollama
    /// server the user also had configured.
    async fn provider_id_for_endpoint(&self, slug: &str, endpoint: &str) -> Option<String> {
        let rows: Vec<ProviderEndpointRow> = sqlx::query_as(
            "SELECT id, json_extract(data, '$.kind'), json_extract(data, '$.preset'), \
                    json_extract(data, '$.baseUrl') \
             FROM providers WHERE enabled = 1",
        )
        .fetch_all(self.storage.pool())
        .await
        .ok()?;
        let want = discovery::origin_of(endpoint);
        rows.into_iter()
            .find(|(_, kind, preset, base_url)| {
                let slug_matches = kind.as_deref() == Some(slug) || preset.as_deref() == Some(slug);
                let endpoint_matches = base_url
                    .as_deref()
                    .is_some_and(|url| discovery::origin_of(url) == want);
                slug_matches && endpoint_matches
            })
            .map(|(id, _, _, _)| id)
    }

    async fn endpoint_for(&self, slug: &str) -> String {
        let record = self.records.read().await.get(slug).cloned();
        // An online record carries the URL the scan actually answered on.
        // A stopped one is ambiguous: `ingest_scan` fills the runtime default,
        // so the provider's configured base URL (custom port, LAN box) has to
        // win or start() would launch a second server on the default port.
        if let Some(record) = &record {
            if record.online() {
                return record.endpoint.clone();
            }
        }
        if let Ok(Some(base_url)) = self.provider_base_url(slug).await {
            return base_url;
        }
        if let Some(record) = record {
            return record.endpoint;
        }
        RuntimeId::from_slug(slug)
            .map(discovery::default_base_url)
            .unwrap_or_else(|| {
                format!(
                    "http://127.0.0.1:{}/v1",
                    discovery::default_port(RuntimeId::Ollama)
                )
            })
    }

    /// First configured endpoint per managed runtime, for the health sweep.
    ///
    /// Kept in one query so the 10-second tick does not hit `providers` once
    /// per installed runtime.
    async fn managed_provider_endpoints(&self) -> HashMap<String, String> {
        let rows: Vec<ProviderEndpointRow> = sqlx::query_as(
            "SELECT id, json_extract(data, '$.kind'), json_extract(data, '$.preset'), \
                    json_extract(data, '$.baseUrl') \
             FROM providers WHERE enabled = 1 ORDER BY created_at ASC",
        )
        .fetch_all(self.storage.pool())
        .await
        .unwrap_or_default();
        let mut endpoints = HashMap::new();
        for (_, kind, preset, base_url) in rows {
            let Some(url) = base_url.filter(|url| !url.trim().is_empty()) else {
                continue;
            };
            if !is_managed_provider(kind.as_deref(), preset.as_deref(), Some(&url)) {
                continue;
            }
            let Some(slug) = preset.or(kind) else {
                continue;
            };
            endpoints.entry(slug).or_insert(url);
        }
        endpoints
    }

    async fn provider_base_url(&self, slug: &str) -> anyhow::Result<Option<String>> {
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT json_extract(data, '$.kind'), json_extract(data, '$.preset'), \
                    json_extract(data, '$.baseUrl') \
             FROM providers WHERE enabled = 1 ORDER BY created_at ASC",
        )
        .fetch_all(self.storage.pool())
        .await?;
        Ok(rows
            .into_iter()
            .find(|(kind, preset, base_url)| {
                (kind.as_deref() == Some(slug) || preset.as_deref() == Some(slug))
                    && is_managed_provider(kind.as_deref(), preset.as_deref(), base_url.as_deref())
            })
            .and_then(|(_, _, url)| url)
            .filter(|url| !url.trim().is_empty()))
    }

    /// Resolve the GUI app bundle for a runtime the app knows about.
    ///
    /// Taking a slug rather than a path keeps an arbitrary WebView-supplied
    /// `.exe`/`.app` from being handed to the OS handler.
    pub async fn open_app(&self, slug: &str) -> anyhow::Result<PathBuf> {
        let runtime = RuntimeId::from_slug(slug)
            .ok_or_else(|| error("unknown_runtime", &[slug.to_string()]))?;
        // The app bundle is the only thing that can be "opened as an app".
        // The record's install hint may be the CLI executable on PATH (the
        // common CLI-only install), which `ensure_open_target` rightly rejects.
        let record_hint = self
            .records
            .read()
            .await
            .get(slug)
            .and_then(|record| record.install_hint.clone())
            .map(PathBuf::from)
            .filter(|path| path.exists());
        let target = discovery::open_app_target(runtime)
            .or(record_hint)
            .ok_or_else(|| simple("open_app_missing"))?;
        ensure_open_target(&target.display().to_string())
    }

    // ------------------------------------------------------------------
    // Model operations
    // ------------------------------------------------------------------

    pub async fn pull(self: &Arc<Self>, slug: &str, model: &str) -> anyhow::Result<String> {
        validate_model_name(model)?;
        let runtime = RuntimeId::from_slug(slug)
            .ok_or_else(|| error("unknown_runtime", &[slug.to_string()]))?;
        // Two clicks on Pull must not download the same tag twice.
        if let Some(existing) = self.running_job(slug, "pull", model) {
            return Ok(existing);
        }
        let id = self.start_job(slug, "pull", model);
        let manager = self.clone();
        let runtime_slug = slug.to_string();
        let model = model.to_string();
        let job_id = id.clone();
        tauri::async_runtime::spawn(async move {
            let endpoint = manager.endpoint_for(&runtime_slug).await;
            let cancel = manager.job_cancel_token(&job_id);
            let result = match runtime {
                RuntimeId::Ollama => {
                    let progress_manager = manager.clone();
                    let progress_job = job_id.clone();
                    local_ops::ollama_pull(&endpoint, &model, cancel, move |progress| {
                        progress_manager.job_progress(&progress_job, progress);
                    })
                    .await
                }
                RuntimeId::LmStudio => {
                    manager.job_message(&job_id, "downloading");
                    local_ops::lmstudio_pull(&model, cancel).await
                }
                _ => Err(error(
                    "pull_unsupported",
                    std::slice::from_ref(&runtime_slug),
                )),
            };
            manager.finish_job(&job_id, result).await;
            if manager
                .jobs
                .lock()
                .ok()
                .and_then(|jobs| jobs.get(&job_id).map(|job| job.view.status.clone()))
                .is_some_and(|status| status == "success")
            {
                discovery::invalidate_lmstudio_cli_cache();
                let _ = manager.refresh_models(&runtime_slug).await;
            }
        });
        Ok(id)
    }

    pub async fn delete_model(&self, slug: &str, model: &str) -> anyhow::Result<()> {
        validate_model_name(model)?;
        match RuntimeId::from_slug(slug) {
            Some(RuntimeId::Ollama) => {
                let endpoint = self.endpoint_for(slug).await;
                local_ops::ollama_delete(&endpoint, model).await?;
            }
            Some(RuntimeId::LmStudio) => {
                return Err(error("delete_unsupported", &["LM Studio".to_string()]))
            }
            _ => return Err(error("delete_unsupported", &[slug.to_string()])),
        }
        discovery::invalidate_lmstudio_cli_cache();
        self.refresh_models(slug).await?;
        Ok(())
    }

    pub async fn load_model(&self, slug: &str, model: &str) -> anyhow::Result<()> {
        validate_model_name(model)?;
        match RuntimeId::from_slug(slug) {
            Some(RuntimeId::Ollama) => {
                let endpoint = self.endpoint_for(slug).await;
                let keep_alive = read_settings(self.storage.pool())
                    .await
                    .ok()
                    .and_then(|settings| settings.local_keep_alive)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "5m".to_string());
                local_ops::ollama_load(&endpoint, model, &keep_alive).await?;
            }
            Some(RuntimeId::LmStudio) => local_ops::lmstudio_load(model).await?,
            _ => return Err(error("load_unsupported", &[slug.to_string()])),
        }
        discovery::invalidate_lmstudio_cli_cache();
        self.refresh_models(slug).await?;
        Ok(())
    }

    pub async fn unload_model(&self, slug: &str, model: &str) -> anyhow::Result<()> {
        validate_model_name(model)?;
        match RuntimeId::from_slug(slug) {
            Some(RuntimeId::Ollama) => {
                let endpoint = self.endpoint_for(slug).await;
                local_ops::ollama_unload(&endpoint, model).await?;
            }
            Some(RuntimeId::LmStudio) => local_ops::lmstudio_unload(model).await?,
            _ => return Err(error("unload_unsupported", &[slug.to_string()])),
        }
        self.refresh_models(slug).await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Jobs
    // ------------------------------------------------------------------

    fn start_job(&self, slug: &str, kind: &str, model: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let view = JobView {
            id: id.clone(),
            runtime: slug.to_string(),
            kind: kind.to_string(),
            model: model.to_string(),
            status: "running".to_string(),
            completed: 0,
            total: 0,
            message: "starting".to_string(),
            error: None,
        };
        let now = Instant::now();
        if let Ok(mut jobs) = self.jobs.lock() {
            prune_jobs(&mut jobs, now);
            jobs.insert(
                id.clone(),
                Job {
                    view: view.clone(),
                    cancel: CancelToken::new(),
                    started: now,
                    finished: None,
                },
            );
        }
        let _ = self.app.emit("local_job_event", view);
        id
    }

    fn running_job(&self, slug: &str, kind: &str, model: &str) -> Option<String> {
        self.jobs.lock().ok().and_then(|jobs| {
            jobs.values()
                .find(|job| {
                    job.view.runtime == slug
                        && job.view.kind == kind
                        && job.view.model == model
                        && matches!(job.view.status.as_str(), "running" | "cancelling")
                })
                .map(|job| job.view.id.clone())
        })
    }

    fn job_cancel_token(&self, id: &str) -> CancelToken {
        self.jobs
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(id).map(|job| job.cancel.clone()))
            .unwrap_or_default()
    }

    fn job_progress(&self, id: &str, progress: PullProgress) {
        let view = self.jobs.lock().ok().and_then(|mut jobs| {
            let job = jobs.get_mut(id)?;
            // A cancel request may land between two progress lines. Writing
            // "running" here would resurrect the spinner and make the next
            // cancel call report "already finished".
            if job.view.status == "cancelling" || job.cancel.is_cancelled() {
                return None;
            }
            job.view.completed = progress.completed;
            job.view.total = progress.total;
            job.view.message = progress.status;
            Some(job.view.clone())
        });
        if let Some(view) = view {
            let _ = self.app.emit("local_job_event", view);
        }
    }

    fn job_message(&self, id: &str, message: &str) {
        let view = self.jobs.lock().ok().and_then(|mut jobs| {
            let job = jobs.get_mut(id)?;
            job.view.message = message.to_string();
            Some(job.view.clone())
        });
        if let Some(view) = view {
            let _ = self.app.emit("local_job_event", view);
        }
    }

    async fn finish_job(&self, id: &str, result: anyhow::Result<()>) {
        let view = self.jobs.lock().ok().and_then(|mut jobs| {
            let job = jobs.get_mut(id)?;
            let cancelled = job.cancel.is_cancelled();
            match &result {
                Ok(()) => {
                    job.view.status = "success".to_string();
                    job.view.message = "complete".to_string();
                    job.view.completed = job.view.total;
                    job.view.error = None;
                }
                Err(_) if cancelled => {
                    job.view.status = "cancelled".to_string();
                    job.view.message = "cancelled".to_string();
                    job.view.error = None;
                }
                Err(err) => {
                    job.view.status = "error".to_string();
                    job.view.message = "failed".to_string();
                    job.view.error = Some(format!("{err:#}"));
                }
            }
            job.finished = Some(Instant::now());
            Some(job.view.clone())
        });
        if let Some(view) = view {
            let _ = self.app.emit("local_job_event", view);
        }
    }

    /// Ask a running job to stop.
    ///
    /// Returns `false` for an unknown or already-terminal job: the old version
    /// reported success for a finished download and left the UI showing a
    /// spinner over an "error" badge.
    pub fn cancel_job(&self, id: &str) -> bool {
        let view = self.jobs.lock().ok().and_then(|mut jobs| {
            let job = jobs.get_mut(id)?;
            if !matches!(job.view.status.as_str(), "running" | "cancelling") {
                return None;
            }
            job.cancel.cancel();
            job.view.status = "cancelling".to_string();
            Some(job.view.clone())
        });
        if let Some(view) = view {
            let _ = self.app.emit("local_job_event", view);
            true
        } else {
            false
        }
    }

    pub fn list_jobs(&self) -> Vec<JobView> {
        let now = Instant::now();
        let Ok(mut jobs) = self.jobs.lock() else {
            return Vec::new();
        };
        prune_jobs(&mut jobs, now);
        // HashMap iteration order is arbitrary; the panel takes the newest
        // three, so the list has to be ordered by start time.
        let mut jobs: Vec<&Job> = jobs.values().collect();
        jobs.sort_by_key(|job| job.started);
        jobs.into_iter().map(|job| job.view.clone()).collect()
    }

    // ------------------------------------------------------------------
    // Startup, health, shutdown
    // ------------------------------------------------------------------

    pub async fn startup(self: &Arc<Self>) {
        let settings = read_settings(self.storage.pool()).await.unwrap_or_default();
        if settings.local_scan_on_start {
            // Keep `initialized` false after a failed scan so the first
            // snapshot can retry; a successful scan (or the opt-out) closes
            // the door so the sweep does not repeat on every poll.
            if self.load_once().await {
                self.initialized.store(true, Ordering::Relaxed);
            }
        } else {
            self.initialized.store(true, Ordering::Relaxed);
        }

        if settings.local_auto_start {
            for slug in self.local_provider_slugs().await {
                if self.provider_auto_start(&slug, &settings).await {
                    let _ = self.start(&slug).await;
                }
            }
        }
        self.spawn_health();
    }

    async fn local_provider_slugs(&self) -> Vec<String> {
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT json_extract(data, '$.preset'), json_extract(data, '$.kind'), \
                    json_extract(data, '$.baseUrl') \
             FROM providers WHERE enabled = 1",
        )
        .fetch_all(self.storage.pool())
        .await
        .unwrap_or_default();
        let mut slugs = Vec::new();
        for (preset, kind, base_url) in rows {
            if !is_managed_provider(kind.as_deref(), preset.as_deref(), base_url.as_deref()) {
                // A LAN/GPU-server Ollama must not spawn a local process.
                continue;
            }
            let slug = preset.or(kind).unwrap_or_default();
            // Only a runtime with a launcher can be auto-started; ask the same
            // capability matrix the UI uses instead of listing vendors here.
            let can_launch = RuntimeId::from_slug(&slug)
                .and_then(discovery::launch_spec)
                .is_some();
            if can_launch && !slugs.contains(&slug) {
                slugs.push(slug);
            }
        }
        slugs
    }

    async fn provider_auto_start(&self, slug: &str, settings: &SettingsView) -> bool {
        let rows: Vec<ProviderAutoStartRow> = sqlx::query_as(
            "SELECT json_extract(data, '$.local.autoStart'), \
                        json_extract(data, '$.kind'), json_extract(data, '$.preset'), \
                        json_extract(data, '$.baseUrl') \
                 FROM providers WHERE enabled = 1 ORDER BY created_at ASC",
        )
        .fetch_all(self.storage.pool())
        .await
        .unwrap_or_default();
        for (auto_start, kind, preset, base_url) in rows {
            let slug_matches = kind.as_deref() == Some(slug) || preset.as_deref() == Some(slug);
            if !slug_matches
                || !is_managed_provider(kind.as_deref(), preset.as_deref(), base_url.as_deref())
            {
                continue;
            }
            if let Some(value) = auto_start {
                return value != 0;
            }
        }
        settings.local_auto_start
    }

    fn spawn_health(self: &Arc<Self>) {
        if self.health_started.swap(true, Ordering::Relaxed) {
            return;
        }
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(HEALTH_INTERVAL).await;
                manager.health_tick().await;
            }
        });
    }

    async fn health_tick(&self) {
        let tick = self.health_ticks.fetch_add(1, Ordering::Relaxed);
        // A stopped-but-installed runtime is probed every third tick (30 s):
        // often enough to notice the user starting it from a terminal, without
        // fetching nine model lists every 10 seconds.
        let include_stopped = tick % 3 == 0;
        let targets: Vec<(String, String, bool)> = {
            let records = self.records.read().await;
            records
                .values()
                .filter(|record| record.installed)
                .filter(|record| {
                    !matches!(
                        record.state,
                        RuntimeState::Starting
                            | RuntimeState::Stopping
                            | RuntimeState::NotInstalled
                    )
                })
                .filter(|record| include_stopped || record.state != RuntimeState::Stopped)
                .map(|record| {
                    (
                        record.slug.clone(),
                        record.endpoint.clone(),
                        record.online(),
                    )
                })
                .collect()
        };
        // One query for every runtime instead of `endpoint_for` per slug (which
        // was an N+1 against `providers` every 10 seconds). A stopped record
        // holds the runtime default; its provider may point at a custom port.
        let overrides = self.managed_provider_endpoints().await;
        for (slug, record_endpoint, record_online) in targets {
            let endpoint = if record_online {
                record_endpoint
            } else {
                overrides.get(&slug).cloned().unwrap_or(record_endpoint)
            };
            let probe = discovery::probe_base_url(&endpoint).await;
            let online = probe.as_ref().is_ok_and(|found| found.online);
            let mut changed = false;
            let mut recovered = false;
            {
                let mut records = self.records.write().await;
                let Some(record) = records.get_mut(&slug) else {
                    continue;
                };
                if online {
                    record.failures = 0;
                    if record.state != RuntimeState::Online {
                        record.state = RuntimeState::Online;
                        record.error = None;
                        recovered = true;
                        changed = true;
                    }
                } else {
                    record.failures = record.failures.saturating_add(1);
                    if record.failures >= 2 && record.state == RuntimeState::Online {
                        record.state = RuntimeState::Degraded;
                        record.error = Some(format!("{:#}", simple("health_failed")));
                        changed = true;
                    }
                }
            }
            if changed {
                self.emit_runtime(&slug).await;
            }
            if recovered {
                // A runtime the user just started has a model list the scan
                // never saw; refresh it instead of showing an empty table.
                let _ = self.refresh_models(&slug).await;
            }
        }
    }

    pub async fn shutdown(&self) {
        let settings = read_settings(self.storage.pool()).await.unwrap_or_default();
        if !settings.local_stop_on_exit {
            return;
        }
        let slugs: Vec<String> = {
            let records = self.records.read().await;
            records
                .values()
                .filter(|record| record.managed)
                .map(|record| record.slug.clone())
                .collect()
        };
        // Stop in parallel so a hung child cannot turn quit into N × 5 s on the
        // main thread. The caller wraps this in one overall deadline.
        let stops = slugs.iter().map(|slug| async move {
            let lock = self.slug_lock(slug).await;
            let _guard = lock.lock().await;
            let _ = self.stop_owned(slug, false).await;
        });
        join_all(stops).await;
    }
}

/// Drop `default_model_id` when it points at a model that no longer exists.
///
/// Deleting the selected model from a runtime is a normal action in the
/// manager. Without this the Composer kept the stale id and the next send
/// failed with "model not found" until the user picked another one.
pub async fn clear_missing_default_model(pool: &sqlx::Pool<sqlx::Sqlite>) {
    let Ok(settings) = read_settings(pool).await else {
        return;
    };
    let Some(current) = settings.default_model_id.clone() else {
        return;
    };
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE id = ?")
        .bind(&current)
        .fetch_one(pool)
        .await
        .unwrap_or(1);
    if exists > 0 {
        return;
    }
    let mut next = settings;
    next.default_model_id = None;
    match crate::commands::settings::write_settings(pool, next).await {
        Ok(_) => {
            tracing::info!(model = %current, "cleared a default model that no longer exists")
        }
        Err(err) => {
            tracing::warn!(%err, model = %current, "could not clear a deleted default model")
        }
    }
}

fn spawn_log_reader<R>(reader: R, logs: Arc<StdMutex<VecDeque<String>>>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            push_log(&logs, line);
        }
    });
}

fn push_log(logs: &StdMutex<VecDeque<String>>, line: String) {
    if let Ok(mut logs) = logs.lock() {
        logs.push_back(line);
        while logs.len() > LOG_LIMIT {
            logs.pop_front();
        }
    }
}

/// Point the launch command at the endpoint the provider actually uses.
///
/// `launch_spec` is built from the runtime's default port. A user who runs
/// Ollama on `11500` would otherwise get a second server on `11434`, and the
/// manager would wait on the wrong port.
fn retarget_spec(
    mut spec: RuntimeLaunchSpec,
    runtime: RuntimeId,
    endpoint: &str,
) -> RuntimeLaunchSpec {
    match runtime {
        RuntimeId::Ollama => {
            let authority = discovery::origin_of(endpoint)
                .split("://")
                .nth(1)
                .unwrap_or_default()
                .to_string();
            if !authority.is_empty() {
                spec.env.retain(|(key, _)| key != "OLLAMA_HOST");
                spec.env.push(("OLLAMA_HOST".into(), authority));
            }
        }
        RuntimeId::LmStudio => {
            let port = url::Url::parse(endpoint)
                .ok()
                .and_then(|url| url.port())
                .unwrap_or_else(|| discovery::default_port(runtime));
            if let Some(index) = spec.args.iter().position(|arg| arg == "--port") {
                match spec.args.get_mut(index + 1) {
                    Some(value) => *value = port.to_string(),
                    None => spec.args.push(port.to_string()),
                }
            } else {
                spec.args.push("--port".into());
                spec.args.push(port.to_string());
            }
        }
        _ => {}
    }
    spec
}

async fn run_cli(spec: &RuntimeLaunchSpec) -> anyhow::Result<()> {
    let output = tokio::process::Command::new(&spec.program)
        .args(&spec.args)
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        .output()
        .await
        .map_err(|err| {
            error(
                "cli_spawn_failed",
                &[spec.program.display().to_string(), err.to_string()],
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(error(
            "cli_failed",
            &[
                spec.program.display().to_string(),
                if stderr.is_empty() { stdout } else { stderr },
            ],
        ));
    }
    Ok(())
}

async fn terminate_child(mut handle: ChildHandle) {
    #[cfg(unix)]
    if let Some(pid) = handle.child.id() {
        // SIGTERM first; Ollama flushes its model list and closes the port.
        // The unsafe call is a single libc signal to a PID we own.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = handle.child.start_kill();
    }
    if tokio::time::timeout(Duration::from_secs(5), handle.child.wait())
        .await
        .is_err()
    {
        let _ = handle.child.start_kill();
        let _ = handle.child.wait().await;
    }
}

async fn wait_online(endpoint: &str, timeout: Duration) -> anyhow::Result<DetectedRuntime> {
    let deadline = Instant::now() + timeout;
    let mut last_error: Option<String>;
    loop {
        match discovery::probe_base_url(endpoint).await {
            Ok(found) if found.online => return Ok(found),
            Ok(found) => {
                last_error = Some(
                    found
                        .error
                        .unwrap_or_else(|| "the model list is still empty".to_string()),
                );
            }
            Err(err) => last_error = Some(err.to_string()),
        }
        if Instant::now() >= deadline {
            anyhow::bail!("{}", last_error.unwrap_or_default());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn detailed_models(endpoint: &str, fallback: DetectedRuntime) -> Vec<DiscoveredModel> {
    match tokio::time::timeout(
        Duration::from_secs(8),
        discovery::probe_base_url_detailed(endpoint),
    )
    .await
    {
        Ok(Ok(found)) => found.models,
        _ => fallback.models,
    }
}

/// Reject names that could be interpreted as a CLI flag or traverse a path.
/// Local model names legitimately contain `/`, `:`, `@`, `.` and `-`.
fn validate_model_name(model: &str) -> anyhow::Result<()> {
    let model = model.trim();
    if model.is_empty() {
        return Err(simple("model_name_empty"));
    }
    if model.len() > 256 {
        return Err(simple("model_name_too_long"));
    }
    if model.starts_with('-') || model.contains("..") || model.contains('\0') {
        return Err(simple("model_name_invalid"));
    }
    if !model
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | ':' | '@'))
    {
        return Err(simple("model_name_invalid"));
    }
    Ok(())
}

/// Path check for the GUI-app button. The path comes from the manager's own
/// record (by slug), so this only guards against a stale install; the command
/// no longer accepts an arbitrary WebView-supplied path.
pub fn ensure_open_target(path: &str) -> anyhow::Result<std::path::PathBuf> {
    let path = Path::new(path);
    if !path.exists() {
        return Err(error("open_app_missing", &[path.display().to_string()]));
    }
    // Opening an arbitrary file through the OS handler is not the same as
    // launching an app. Restrict this command to app bundles/directories and
    // executable installers; the scan never passes anything else.
    let allowed = path.is_dir()
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "app" | "exe" | "appimage" | "desktop"
                )
            });
    if !allowed {
        return Err(error("open_app_denied", &[path.display().to_string()]));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_open_target, model_operations, prune_jobs, retarget_spec, validate_model_name, Job,
        JobView, JOB_TTL,
    };
    use mixd_providers::discovery::{RuntimeId, RuntimeLaunchSpec, RuntimeStop};
    use mixd_providers::local_ops::CancelToken;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn spec() -> RuntimeLaunchSpec {
        RuntimeLaunchSpec {
            program: PathBuf::from("/usr/local/bin/lms"),
            args: vec![
                "server".into(),
                "start".into(),
                "--port".into(),
                "1234".into(),
            ],
            env: Vec::new(),
            stop: RuntimeStop::Unsupported,
        }
    }

    #[test]
    fn lmstudio_launch_uses_the_provider_port() {
        let spec = retarget_spec(spec(), RuntimeId::LmStudio, "http://127.0.0.1:1250/v1");
        let port = spec.args.iter().position(|arg| arg == "--port").unwrap();
        assert_eq!(spec.args[port + 1], "1250");
    }

    #[test]
    fn ollama_launch_uses_the_provider_authority() {
        let mut spec = spec();
        spec.program = PathBuf::from("/usr/local/bin/ollama");
        spec.args = vec!["serve".into()];
        spec.env = vec![("OLLAMA_HOST".into(), "127.0.0.1:11434".into())];
        let spec = retarget_spec(spec, RuntimeId::Ollama, "http://127.0.0.1:11500/v1");
        assert_eq!(
            spec.env,
            vec![("OLLAMA_HOST".to_string(), "127.0.0.1:11500".to_string())]
        );
    }

    #[test]
    fn open_target_rejects_files_that_are_not_apps() {
        let path = std::env::temp_dir().join("mix-agent-not-an-app.txt");
        std::fs::write(&path, b"x").expect("write temp file");
        let err = ensure_open_target(&path.display().to_string()).expect_err("rejected");
        assert!(format!("{err:#}").starts_with("mix-local-error:open_app_denied"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn model_names_allow_local_tag_and_hub_shapes() {
        for name in [
            "llama3.1:8b",
            "qwen/qwen3-8b",
            "qwen/qwen3.5-9b@q8_0",
            "text-embedding-nomic-embed-text-v1.5",
        ] {
            assert!(validate_model_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn model_names_reject_flags_and_traversal() {
        for name in ["--help", "-y", "../secret", "a\rb", "", "a b"] {
            assert!(validate_model_name(name).is_err(), "{name}");
        }
    }

    fn job_with_finish(finished: Option<Instant>) -> Job {
        Job {
            view: JobView {
                id: "job".into(),
                runtime: "ollama".into(),
                kind: "pull".into(),
                model: "qwen3:8b".into(),
                status: if finished.is_some() {
                    "success"
                } else {
                    "running"
                }
                .into(),
                completed: 1,
                total: 1,
                message: String::new(),
                error: None,
            },
            cancel: CancelToken::new(),
            started: Instant::now(),
            finished,
        }
    }

    #[test]
    fn model_operations_are_one_matrix_for_ui_and_snapshot() {
        let ollama = model_operations("ollama");
        assert!(ollama.pull && ollama.delete && ollama.load && ollama.unload);
        let unknown = model_operations("vllm");
        assert!(!unknown.pull && !unknown.delete && !unknown.load && !unknown.unload);
    }

    #[tokio::test]
    async fn a_deleted_default_model_is_cleared_but_a_live_one_is_kept() {
        use crate::commands::settings::{read_settings, write_settings, SettingsView};
        use mixd_storage::Storage;

        let dir = std::env::temp_dir().join(format!("mixd-default-test-{}", uuid::Uuid::new_v4()));
        let storage = Storage::open(&dir).await.expect("open storage");
        let mut settings = SettingsView {
            default_model_id: Some("provider-1:gone".into()),
            ..SettingsView::default()
        };
        write_settings(storage.pool(), settings.clone())
            .await
            .expect("write settings");

        super::clear_missing_default_model(storage.pool()).await;
        assert_eq!(
            read_settings(storage.pool())
                .await
                .expect("read settings")
                .default_model_id,
            None
        );

        sqlx::query("INSERT INTO models (id, owner_id, data) VALUES (?, ?, ?)")
            .bind("provider-1:kept")
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(r#"{"providerId":"provider-1","label":"kept"}"#)
            .execute(storage.pool())
            .await
            .expect("insert model");
        settings.default_model_id = Some("provider-1:kept".into());
        write_settings(storage.pool(), settings)
            .await
            .expect("write settings");
        super::clear_missing_default_model(storage.pool()).await;
        assert_eq!(
            read_settings(storage.pool())
                .await
                .expect("read settings")
                .default_model_id
                .as_deref(),
            Some("provider-1:kept")
        );
    }

    #[test]
    fn terminal_jobs_are_pruned_after_the_ttl() {
        let now = Instant::now();
        let mut jobs = HashMap::new();
        jobs.insert(
            "old".to_string(),
            job_with_finish(Some(now - JOB_TTL - Duration::from_secs(1))),
        );
        jobs.insert("new".to_string(), job_with_finish(Some(now)));
        jobs.insert("running".to_string(), job_with_finish(None));
        prune_jobs(&mut jobs, now);
        assert!(!jobs.contains_key("old"));
        assert!(jobs.contains_key("new"));
        assert!(jobs.contains_key("running"));
    }
}
