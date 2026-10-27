//! Local runtime detection.
//!
//! `scan_local_runtimes` answers "is LM Studio (or Ollama, vLLM, llama.cpp, …)
//! running on this machine?". `import_local_runtime` turns an answer into a
//! usable provider: the row is written, the models are synced, and the adapter
//! is registered with the Run Engine — so the models are selectable on the very
//! next message instead of after a restart.
//!
//! The scanning itself lives in [`mixd_providers::discovery`]; this module only
//! owns the trust boundary and the database writes.

use crate::AppState;
use mixd_providers::discovery::{
    self, DetectedRuntime, DiscoveredModel, ModelCapabilities, OccupiedPort,
};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct LocalRuntimeView {
    pub slug: String,
    pub label: String,
    pub kind: String,
    pub base_url: String,
    pub host: String,
    pub port: u16,
    /// The port accepted a connection. `false` for a runtime that is installed
    /// but not serving.
    pub listening: bool,
    /// The port serves a model list, i.e. this is importable.
    pub online: bool,
    pub latency_ms: Option<u64>,
    pub version: Option<String>,
    pub installed: bool,
    pub install_hint: Option<String>,
    pub launcher: Option<String>,
    pub models: Vec<LocalModelView>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct LocalModelView {
    pub id: String,
    pub label: String,
    pub context_window: u32,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub size_bytes: Option<u64>,
    pub family: Option<String>,
    pub loaded: bool,
    pub model_type: Option<String>,
    pub capabilities: Option<ModelCapabilities>,
}

#[derive(Serialize)]
pub struct OccupiedPortView {
    pub port: u16,
    pub note: Option<String>,
}

#[derive(Serialize)]
pub struct LocalScanView {
    pub runtimes: Vec<LocalRuntimeView>,
    pub occupied_ports: Vec<OccupiedPortView>,
    pub hosts: Vec<String>,
    pub elapsed_ms: u64,
    /// `true` when at least one runtime answered, so the UI can lead with a
    /// success state instead of a bare list.
    pub any_online: bool,
}

impl From<DetectedRuntime> for LocalRuntimeView {
    fn from(r: DetectedRuntime) -> Self {
        Self {
            slug: r.slug,
            label: r.label,
            kind: r.kind,
            base_url: r.base_url,
            host: r.host,
            port: r.port,
            listening: r.listening,
            online: r.online,
            latency_ms: r.latency_ms,
            version: r.version,
            installed: r.installed,
            install_hint: r.install_hint,
            launcher: r.launcher,
            models: r.models.into_iter().map(LocalModelView::from).collect(),
            error: r.error,
        }
    }
}

impl From<OccupiedPort> for OccupiedPortView {
    fn from(o: OccupiedPort) -> Self {
        Self {
            port: o.port,
            note: o.note,
        }
    }
}

impl From<DiscoveredModel> for LocalModelView {
    fn from(m: DiscoveredModel) -> Self {
        Self {
            id: m.id,
            label: m.label,
            context_window: m.context_window,
            parameter_size: m.parameter_size,
            quantization: m.quantization,
            size_bytes: m.size_bytes,
            family: m.family,
            loaded: m.loaded,
            model_type: m.model_type,
            capabilities: m.capabilities,
        }
    }
}

/// Sweep the loopback interface for local inference runtimes.
///
/// Never fails on a closed port: those are reported by omission. A transport
/// failure that affects every probe at once (no client, no permissions) is the
/// only case that surfaces as an error.
#[tauri::command]
pub async fn scan_local_runtimes() -> Result<LocalScanView, String> {
    // The UI's detect button is the one place a user expects complete rows:
    // context length and capability badges are what make the import decision.
    // The fast sweep remains available to internal callers.
    let report = discovery::scan_local_runtimes_detailed()
        .await
        .map_err(crate::error::cmd_err)?;
    let any_online = report.runtimes.iter().any(|r| r.online);
    Ok(LocalScanView {
        runtimes: report
            .runtimes
            .into_iter()
            .map(LocalRuntimeView::from)
            .collect(),
        occupied_ports: report
            .occupied_ports
            .into_iter()
            .map(OccupiedPortView::from)
            .collect(),
        hosts: report.hosts,
        elapsed_ms: report.elapsed_ms,
        any_online,
    })
}

/// Probe one base URL the user typed, for a runtime on another machine, an
/// unusual port, or a runtime not in the sweep table.
#[tauri::command]
pub async fn probe_local_endpoint(base_url: String) -> Result<LocalRuntimeView, String> {
    let runtime = discovery::probe_base_url_detailed(&base_url)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(LocalRuntimeView::from(runtime))
}

#[derive(Deserialize)]
pub struct ImportInput {
    pub slug: String,
    pub kind: String,
    pub label: String,
    pub base_url: String,
    /// The runtime's model list, as shown in the scan result. Re-probing here
    /// would make the button feel slower for no gain, and the server may have
    /// been stopped between the scan and the click.
    pub models: Vec<DiscoveredModel>,
}

#[derive(Serialize)]
pub struct ImportView {
    pub provider: crate::commands::providers::ProviderView,
    /// Models written by this import, in the same order the scan reported.
    pub models: Vec<crate::commands::models::ModelView>,
}

/// Register a detected runtime as a provider and sync its model list.
#[tauri::command]
pub async fn import_local_runtime(
    state: State<'_, AppState>,
    input: ImportInput,
) -> Result<ImportView, String> {
    let base_url = input.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err("Base URL が空です".into());
    }
    // The scan result is advisory: it may have been produced by an older
    // client, or by the fast sweep before native details existed. Re-read the
    // runtime's own metadata here so the rows written to the model table carry
    // the context window and capability flags the Run Engine needs. A failure
    // is not fatal — the import still works with whatever the scan supplied.
    let mut models = input.models;
    let _ = discovery::enrich_models_by_slug(&input.slug, &base_url, &mut models).await;
    // `insert_provider` writes the row *and* puts the adapter in the live
    // registry, which is what makes the models synced below usable on the very
    // next message instead of after a restart.
    let provider = crate::commands::providers::insert_provider(
        &state,
        Some(input.slug),
        &input.kind,
        &input.label,
        &base_url,
        None,
        Some(true),
    )
    .await?;

    let models = crate::commands::models::sync_models(&state, &provider.id, &models).await?;
    Ok(ImportView { provider, models })
}
