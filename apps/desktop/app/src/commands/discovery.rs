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

use crate::commands::settings::{read_settings, write_settings};
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
    /// The runtime answered and is identified. An empty library is online but
    /// not importable until at least one model exists.
    pub online: bool,
    pub latency_ms: Option<u64>,
    pub version: Option<String>,
    pub installed: bool,
    pub install_hint: Option<String>,
    pub launcher: Option<String>,
    /// A GUI bundle the manager can open by slug. `install_hint` may point at
    /// the CLI executable instead, which must not be handed to the OS handler.
    pub can_open_app: bool,
    /// Capability flags from the same matrix the manager uses, so this table
    /// never hardcodes a vendor slug.
    pub can_start: bool,
    pub can_pull: bool,
    pub can_delete: bool,
    pub can_load: bool,
    pub can_unload: bool,
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
        let runtime = discovery::RuntimeId::from_slug(&r.slug);
        let can_open_app = runtime.and_then(discovery::open_app_target).is_some();
        let can_start = runtime.and_then(discovery::launch_spec).is_some();
        let operations = crate::local_runtime::model_operations(&r.slug);
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
            can_open_app,
            can_start,
            can_pull: operations.pull,
            can_delete: operations.delete,
            can_load: operations.load,
            can_unload: operations.unload,
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
pub async fn scan_local_runtimes(state: State<'_, AppState>) -> Result<LocalScanView, String> {
    // The UI's detect button is the one place a user expects complete rows:
    // context length and capability badges are what make the import decision.
    // The fast sweep remains available to internal callers.
    let report = discovery::scan_local_runtimes_detailed()
        .await
        .map_err(crate::error::cmd_err)?;
    // The lifecycle manager is the single source of truth for state after
    // this: the scan is also what teaches it which installed runtimes exist.
    state.local_runtime.ingest_scan(&report).await;
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
///
/// The URL is user-controlled and the backend fetches it, so it is confined to
/// loopback/private/link-local(LAN) addresses. Without this a compromised
/// renderer could use the app as an SSRF proxy against cloud metadata or the
/// public internet, and the response would never be shown to the user.
#[tauri::command]
pub async fn probe_local_endpoint(base_url: String) -> Result<LocalRuntimeView, String> {
    validate_local_probe(&base_url).await?;
    let runtime = discovery::probe_base_url_detailed(base_url.trim())
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(LocalRuntimeView::from(runtime))
}

fn allowed_local_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_unspecified()
            // Link-local (169.254.0.0/16) is deliberately excluded: it hosts
            // cloud metadata endpoints and self-assigned interfaces are not a
            // supported local-model address.
        }
        std::net::IpAddr::V6(v6) => {
            // Unique-local (fc00::/7) and link-local (fe80::/10) are the IPv6
            // equivalents of a LAN address. Written with masks rather than
            // `is_unique_local` so the crate keeps its 1.82 MSRV.
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

async fn validate_local_probe(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw.trim()).map_err(|_| "URLの形式が正しくありません".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("http または https のURLを指定してください".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLに認証情報を含めることはできません".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URLにホスト名がありません".to_string())?;
    let port = url.port_or_known_default().unwrap_or(80);
    let Ok(addrs) = tokio::net::lookup_host((host, port)).await else {
        // A name that does not resolve cannot reach anything; let the probe
        // report the connection failure with its own context.
        return Ok(());
    };
    let resolved: Vec<_> = addrs.collect();
    if resolved.is_empty() {
        return Ok(());
    }
    if resolved.iter().all(|addr| allowed_local_ip(addr.ip())) {
        Ok(())
    } else {
        Err("ローカルまたはプライベートネットワークのURLのみ指定できます".to_string())
    }
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
    let slug = input.slug;
    let kind = input.kind;
    let label = input.label;
    let mut models = input.models;
    let _ = discovery::enrich_models_by_slug(&slug, &base_url, &mut models).await;

    // Re-importing the same endpoint is a refresh, not a duplicate provider.
    // The old path failed on the duplicate label, which made "scan again after
    // a pull" the one workflow that could not work.
    let existing: Option<(String, String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.kind'), json_extract(data, '$.label'), \
                json_extract(data, '$.preset'), \
                COALESCE(json_extract(data, '$.enabled'), 1) \
         FROM providers \
         WHERE json_extract(data, '$.kind') = ? \
           AND json_extract(data, '$.baseUrl') = ? \
         LIMIT 1",
    )
    .bind(&kind)
    .bind(&base_url)
    .fetch_optional(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;

    let provider = if let Some((id, stored_kind, stored_label, stored_preset, enabled)) = existing {
        // Keep the user's rename and their enable/disable choice. The returned
        // view has to mirror the database: registering a disabled provider and
        // reporting `enabled: true` used to leave the registry and the pane
        // disagreeing.
        let enabled = enabled != 0;
        if enabled {
            crate::register_adapter(
                &state.engine.providers,
                state.secrets.as_ref(),
                &id,
                &stored_kind,
                &base_url,
                true,
            )
            .await;
        }
        crate::commands::providers::ProviderView {
            id,
            kind: stored_kind,
            label: stored_label,
            base_url: Some(base_url.clone()),
            enabled,
            has_api_key: false,
            preset: stored_preset.or(Some(slug.clone())),
        }
    } else {
        // `insert_provider` writes the row *and* puts the adapter in the live
        // registry, which is what makes the models synced below usable on the
        // very next message instead of after a restart.
        crate::commands::providers::insert_provider(
            &state,
            Some(slug.clone()),
            &kind,
            &label,
            &base_url,
            None,
            Some(true),
        )
        .await?
    };

    // A first-ever model import may choose a default; an import into an
    // existing library must not override the user's "Auto" choice.
    let had_models: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models")
        .fetch_one(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    let models =
        crate::commands::models::sync_models(state.storage.pool(), &provider.id, &models).await?;
    if had_models == 0 {
        ensure_default_model(&state, &models).await;
    }
    Ok(ImportView { provider, models })
}

/// Point the app at the first imported model when the user has not chosen one.
///
/// A local provider with models but no default used to land in the Composer
/// with "auto" and no obvious next step. An existing default is never touched.
async fn ensure_default_model(state: &AppState, models: &[crate::commands::models::ModelView]) {
    if models.is_empty() {
        return;
    }
    let Ok(settings) = read_settings(state.storage.pool()).await else {
        return;
    };
    if settings.default_model_id.is_some() {
        return;
    }
    let Some(selected) = models
        .iter()
        .find(|model| model.loaded)
        .or_else(|| models.first())
    else {
        return;
    };
    let mut next = settings;
    next.default_model_id = Some(selected.id.clone());
    if write_settings(state.storage.pool(), next).await.is_ok() {
        state.embeddings.invalidate();
    }
}
