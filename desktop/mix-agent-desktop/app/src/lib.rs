//! MIX-agent Desktop application library.

pub mod commands;
pub mod deep_link;
pub mod diff;
pub mod error;
pub mod http;
pub mod menu;
pub mod push;
pub mod run_events;
pub mod scheduler;
pub mod tray;
pub mod updater;

use mixd_core::runs::engine::{ProviderRegistry, RunEngine};
use mixd_mcp::McpHost;
use mixd_providers::{
    anthropic::AnthropicProvider, gemini::GeminiProvider, openai::OpenAiProvider,
    preset as preset_catalog, Provider,
};
use mixd_storage::{SecretStore, Storage};
use std::sync::Arc;

/// Bridges the MCP host into the Tool Registry's `mcp__*` dispatcher.
struct McpBridge {
    host: Arc<McpHost>,
}

#[async_trait_impl::async_trait]
impl mixd_tools::McpDispatch for McpBridge {
    async fn dispatch(
        &self,
        qualified_name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<mixd_tools::ToolResult> {
        let result = self
            .host
            .dispatch(qualified_name, arguments)
            .await
            .map_err(anyhow::Error::from)?;
        Ok(mixd_tools::ToolResult {
            ok: !result.is_error,
            content: result.content,
            error: if result.is_error {
                Some("MCP tool reported an error".to_string())
            } else {
                None
            },
        })
    }
}

/// Serves the skill catalogue to the `skill` Tool from the database.
struct DbSkillSource {
    storage: Arc<Storage>,
}

#[async_trait_impl::async_trait]
impl mixd_tools::skill::SkillSource for DbSkillSource {
    async fn skills(
        &self,
        _root: Option<&str>,
    ) -> anyhow::Result<Vec<mixd_tools::skill::SkillSummary>> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT data FROM skills ORDER BY json_extract(data, '$.name')")
                .fetch_all(self.storage.pool())
                .await?;
        Ok(rows
            .iter()
            .filter_map(|raw| serde_json::from_str(raw).ok())
            .map(
                |skill: mixd_core::skills::Skill| mixd_tools::skill::SkillSummary {
                    name: skill.name,
                    description: skill.description,
                    body_markdown: skill.body_markdown,
                    allowed_tools: skill.allowed_tools,
                },
            )
            .collect())
    }
}

/// Serves the saved agent profiles to the `agent` Tool.
struct DbAgentSource {
    storage: Arc<Storage>,
}

#[async_trait_impl::async_trait]
impl mixd_tools::agent::AgentSource for DbAgentSource {
    async fn profiles(&self) -> anyhow::Result<Vec<mixd_tools::agent::AgentProfile>> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, data FROM agents")
            .fetch_all(self.storage.pool())
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, raw)| {
                let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
                Some(mixd_tools::agent::AgentProfile {
                    id,
                    name: value.get("name")?.as_str()?.to_string(),
                    instructions: value
                        .get("instructions")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    tool_ids: value
                        .get("toolIds")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default(),
                })
            })
            .collect())
    }
}

/// Runs a delegated agent as a fresh sub-run of the same engine.
///
/// The engine reference is a back-pointer filled in once construction finishes.
/// A direct `Arc<RunEngine>` would be circular: the engine is built *from* the
/// registry that holds this tool.
#[derive(Default)]
pub(crate) struct EngineAgentRunner {
    engine: std::sync::OnceLock<std::sync::Weak<RunEngine>>,
    model_id: std::sync::Mutex<String>,
}

impl EngineAgentRunner {
    pub(crate) fn attach(&self, engine: &Arc<RunEngine>, model_id: &str) {
        let _ = self.engine.set(Arc::downgrade(engine));
        *self.model_id.lock().expect("model id lock") = model_id.to_string();
    }

    fn engine(&self) -> anyhow::Result<Arc<RunEngine>> {
        self.engine
            .get()
            .and_then(|weak| weak.upgrade())
            .ok_or_else(|| {
                anyhow::anyhow!("agent delegation is unavailable before startup completes")
            })
    }
}

#[async_trait_impl::async_trait]
impl mixd_tools::agent::AgentRunner for EngineAgentRunner {
    async fn delegate(
        &self,
        parent: &mixd_tools::ToolContext,
        profile: &mixd_tools::agent::AgentProfile,
        task: &str,
    ) -> anyhow::Result<mixd_tools::agent::DelegateResult> {
        use mixd_core::runs::engine::RunRequest;
        use mixd_core::runs::state::RunStatus;

        let engine = self.engine()?;
        let model_id = self.model_id.lock().expect("model id lock").clone();
        // `attach` is given a placeholder, and an empty model id is sent
        // verbatim by every adapter — a guaranteed 400. The parent run is the
        // authority: it is the run that invoked this tool.
        let model_id = if model_id.is_empty() {
            parent_model(&engine, parent.run_id).await?
        } else {
            model_id
        };
        let specs: Vec<mixd_tools::ToolSpec> = engine
            .tool_specs()
            .await
            .into_iter()
            .map(|spec| mixd_tools::ToolSpec {
                name: spec.name,
                description: spec.description,
                parameters: spec.parameters,
            })
            .collect();
        let permitted = mixd_tools::agent::AgentTool::permitted(specs, &profile.tool_ids);
        if permitted.is_empty() && !profile.tool_ids.is_empty() {
            anyhow::bail!(
                "profile {:?} permits no available tool (allowed: {})",
                profile.name,
                profile.tool_ids.join(", ")
            );
        }
        // The sub-agent must not delegate again: an unbounded tree of sub-runs
        // is a fork bomb, and a nested agent has no way to summarise better
        // than its parent.
        let permitted: Vec<mixd_providers::ToolSpec> = permitted
            .into_iter()
            .filter(|s| s.name != "agent")
            .map(|spec| mixd_providers::ToolSpec {
                name: spec.name,
                description: spec.description,
                parameters: spec.parameters,
            })
            .collect();

        let sub_run_id = uuid::Uuid::new_v4();
        let system = format!(
            "You are the {:?} agent.\n\n{}\n\nYou are running as a sub-agent: you cannot              see the parent conversation and you cannot delegate further. Complete the task              and report a conclusion the parent can act on. Include concrete file paths and              line numbers.",
            profile.name, profile.instructions
        );
        // Reuse the parent's provider so a sub-agent cannot silently fail for
        // lack of a resolvable model.
        let provider_id = engine
            .providers
            .list()
            .await
            .first()
            .map(|h| h.id.clone())
            .ok_or_else(|| anyhow::anyhow!("no provider is configured"))?;
        let request = RunRequest {
            run_id: sub_run_id,
            provider_id,
            // The sub-run is not part of the visible conversation, so it is
            // keyed to the parent's own run id and never listed on its own.
            conversation_id: parent.run_id,
            workspace_id: parent.workspace_id,
            // Scoped to the parent's folder. `None` means "process CWD", which
            // for a Finder-launched app is `/` — the sub-agent would read and
            // write outside the project.
            root_path: parent.root_path.clone(),
            mode: mixd_core::mode::Mode::Thinking,
            model_id,
            system: Some(system),
            temperature: None,
            reasoning: None,
            max_tokens: None,
            tools: permitted,
            history: Vec::new(),
            // A blank user message keeps the persisted transcript distinguishable
            // from the parent's; the real instruction is the system prompt.
            user_message: task.to_string(),
            resume_from: None,
        };
        let outcome = engine.execute(request).await?;
        Ok(mixd_tools::agent::DelegateResult {
            agent: profile.name.clone(),
            steps: outcome.steps,
            tool_calls: outcome.tool_calls.iter().map(|c| c.name.clone()).collect(),
            text: outcome.text,
            error: match outcome.status {
                RunStatus::Completed | RunStatus::Cancelled => None,
                other => Some(format!("sub-agent ended as {}", other.as_str())),
            },
        })
    }
}

/// Tiny local re-export so the bridges can name the trait without another
/// workspace dependency.
mod async_trait_impl {
    pub use async_trait::async_trait;
}

/// The model the parent Run was launched with, so a sub-agent answers with the
/// same one instead of the empty id `attach` was seeded with.
async fn parent_model(engine: &RunEngine, run_id: uuid::Uuid) -> anyhow::Result<String> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT json_extract(data, '$.modelId') FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_optional(engine.storage.pool())
            .await
            .map_err(|err| anyhow::anyhow!("reading the parent run's model failed: {err}"))?;
    let model = row.and_then(|(m,)| m).filter(|m| !m.trim().is_empty());
    match model {
        Some(model) => Ok(model),
        None => anyhow::bail!("cannot start a sub-agent: the parent run has no model configured"),
    }
}

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Storage>,
    pub secrets: Arc<SecretStore>,
    pub engine: Arc<RunEngine>,
    pub browser: Arc<mixd_browser::BrowserOperator>,
    pub mcp: Arc<McpHost>,
    /// The same runner instance the engine's registry was built with, so a
    /// rebuild after an MCP change keeps the `agent` tool working.
    pub(crate) agent_runner: Arc<EngineAgentRunner>,
}

/// Assemble the Tool Registry from the live parts.
///
/// Two places build a registry — startup and every MCP connect/disconnect — and
/// they have to produce the same tool list. Rebuilding from scratch used to
/// drop the skill and agent tools (and reset the egress allowlist) because only
/// the startup path attached them, which made an MCP operation silently strip
/// the agent of half its capabilities.
pub(crate) fn build_registry(
    storage: &Arc<Storage>,
    browser: &Arc<mixd_browser::BrowserOperator>,
    mcp: &Arc<McpHost>,
    agent_runner: &Arc<EngineAgentRunner>,
    egress: mixd_tools::EgressPolicy,
) -> mixd_tools::ToolRegistry {
    mixd_tools::ToolRegistry::new(egress)
        .with_browser(browser.clone())
        .with_mcp(Arc::new(McpBridge { host: mcp.clone() }))
        // Project skills first: a procedure a repository carries itself is
        // more specific than a stored one with the same name.
        .with_skills(Arc::new(mixd_tools::skill::CompositeSkillSource::new(
            vec![
                Arc::new(mixd_core::skills::discovery::ProjectSkillSource),
                Arc::new(DbSkillSource {
                    storage: storage.clone(),
                }),
            ],
        )))
        .with_agents(
            Arc::new(DbAgentSource {
                storage: storage.clone(),
            }),
            agent_runner.clone(),
        )
}

pub async fn build_app_state(
    storage: Arc<Storage>,
    secrets: Arc<SecretStore>,
    app_data: &std::path::Path,
    resource_dir: Option<std::path::PathBuf>,
) -> anyhow::Result<AppState> {
    // The browser runs headed on desktop builds so the user can watch and
    // interact with what the agent is doing.
    let headless = cfg!(debug_assertions);
    let browser = Arc::new(mixd_browser::BrowserOperator::with_resources(
        app_data,
        resource_dir,
        headless,
    ));

    // Load persisted MCP connections before the Tool Registry is built so the
    // first Run already sees the connected servers.
    let mcp = Arc::new(McpHost::new());
    match mixd_mcp::hydrate_and_connect(&mcp, storage.as_ref(), Some(secrets.as_ref())).await {
        Ok(count) => tracing::info!(count, "MCP servers connected"),
        Err(err) => {
            tracing::warn!(?err, "MCP hydration failed; continuing without MCP tools");
        }
    }
    // The specs have to be pulled explicitly. The registry can dispatch
    // `mcp__*` calls, but without these the model is never told the tools
    // exist, so a fully configured server is unusable.
    let mcp_specs = mixd_mcp::collect_tool_specs(&mcp).await;
    tracing::info!(tools = mcp_specs.len(), "MCP tools advertised");

    let egress = load_egress_policy(storage.as_ref()).await;
    // The agent runner needs the engine, and the engine needs the registry, so
    // the runner is wired in afterwards through a self-reference.
    let agent_runner = Arc::new(EngineAgentRunner::default());
    let mut tools = build_registry(&storage, &browser, &mcp, &agent_runner, egress);
    tools.set_mcp_specs(
        mcp_specs
            .into_iter()
            .map(|spec| mixd_tools::ToolSpec {
                name: spec.name,
                description: spec.description,
                parameters: spec.parameters,
            })
            .collect(),
    );

    let engine = Arc::new(RunEngine::with_storage_and_tools(
        storage.clone(),
        secrets.clone(),
        tools,
    ));
    // Close the cycle now that the engine exists.
    agent_runner.attach(&engine, "");
    register_providers_from_storage(&engine.providers, &storage, &secrets).await?;
    Ok(AppState {
        storage,
        secrets,
        engine,
        browser,
        mcp,
        agent_runner,
    })
}

/// Read the egress allowlist out of `app_settings`.
pub(crate) async fn load_egress_policy(storage: &Storage) -> mixd_tools::EgressPolicy {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT data FROM app_settings WHERE id = 'singleton'")
            .fetch_optional(storage.pool())
            .await
            .unwrap_or(None);
    let allowlist = row
        .and_then(|(raw,)| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("egressAllowlist").cloned())
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default();
    mixd_tools::EgressPolicy::from_allowlist(allowlist)
}

async fn register_providers_from_storage(
    registry: &ProviderRegistry,
    storage: &Storage,
    secrets: &SecretStore,
) -> anyhow::Result<()> {
    let pool = storage.pool();
    let rows: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.kind') AS kind, json_extract(data, '$.baseUrl') AS base_url, \
         json_extract(data, '$.enabled') AS enabled FROM providers",
    )
    .fetch_all(pool)
    .await?;
    for (id, kind, base_url, enabled) in rows {
        if enabled == 0 {
            continue;
        }
        let base_url = base_url.unwrap_or_default();
        let api_key = secrets
            .get_string(&format!("provider/{}", id), "api_key")
            .await
            .ok();
        registry
            .register(id, build_provider(&kind, base_url, api_key))
            .await;
    }
    let _ = preset_catalog::PRESETS; // keep the preset table linked
    Ok(())
}

/// Build the adapter for a stored `kind` string.
///
/// Only the three major providers have a distinct wire format; Ollama, LM
/// Studio, and every other OpenAI-compatible runtime reuse the OpenAI client
/// with a base URL — and, for the local ones, no key. Shared by boot-time
/// registration and by provider creation, so a provider imported mid-session
/// behaves exactly like one that was there at launch.
pub fn build_provider(kind: &str, base_url: String, api_key: Option<String>) -> Arc<dyn Provider> {
    use mixd_providers::preset::ProviderKind;
    match ProviderKind::parse(kind) {
        Some(ProviderKind::Anthropic) => Arc::new(AnthropicProvider::new(base_url, api_key)),
        Some(ProviderKind::Gemini) => Arc::new(GeminiProvider::new(base_url, api_key)),
        Some(ProviderKind::Ollama) => Arc::new(mixd_providers::ollama::ollama(base_url)),
        Some(ProviderKind::LMStudio) => Arc::new(mixd_providers::lmstudio::lmstudio(base_url)),
        _ => Arc::new(OpenAiProvider::new(base_url, api_key)),
    }
}

/// Bundle identifier from `app/tauri.conf.json`.
///
/// Tauri's `PathResolver::app_data_dir` resolves to
/// `dirs::data_dir().join(identifier)`; tracing needs the same directory before
/// the `AppHandle` exists, so the value is repeated here (the test at the
/// bottom of this file keeps it in sync with the config file).
const APP_IDENTIFIER: &str = "io.github.taitai2661.mix-agent-desktop";

/// Console + rolling file logging.
///
/// The file layer writes into the app-data directory with daily rotation, so a
/// crash that takes the WebView down still leaves a trail behind; the console
/// layer keeps the exact behaviour `RUST_LOG` users had before. The directory
/// is resolved through `dirs` rather than `PathResolver` because the
/// subscriber has to be installed before `tauri::Builder` runs. If the
/// directory cannot be created we fall back to console-only instead of losing
/// logging altogether.
fn init_tracing() {
    use tracing_subscriber::layer::{Layer, SubscriberExt};
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let console = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_filter(filter.clone());

    let Some(dir) = dirs::data_dir().map(|base| base.join(APP_IDENTIFIER)) else {
        tracing_subscriber::registry().with(console).init();
        return;
    };
    if let Err(err) = std::fs::create_dir_all(&dir) {
        // No subscriber is installed yet, so `tracing` would swallow this.
        eprintln!(
            "tracing: cannot create the log directory {}: {err}; console logging only",
            dir.display()
        );
        tracing_subscriber::registry().with(console).init();
        return;
    }

    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(&dir, "mix-agent.log"));
    // A `WorkerGuard` must outlive every log record; storing it in a `static`
    // is what keeps the background writer from shutting down mid-process.
    static LOG_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
        std::sync::OnceLock::new();
    let _ = LOG_GUARD.set(guard);

    let file = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(writer)
        .with_filter(filter);
    tracing_subscriber::registry()
        .with(console)
        .with(file)
        .init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_cli::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            use tauri::Manager;
            let app_handle = app.handle().clone();
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("mix-agent"));
            // Use Tauri's global async runtime rather than a throwaway one: MCP
            // stdio sessions spawn long-lived reader tasks, and those tasks
            // would be torn down when a local runtime is dropped at the end of
            // setup, silently killing every connected server.
            let storage = std::sync::Arc::new(
                tauri::async_runtime::block_on(mixd_storage::Storage::open(&data_dir))
                    .expect("open storage"),
            );
            let secrets = std::sync::Arc::new(mixd_storage::SecretStore::new());
            // Tauri's resolved resource dir is where the bundled Chromium was
            // unpacked; guessing it from app_data breaks on installed builds.
            let resource_dir = app.path().resource_dir().ok();
            let state = tauri::async_runtime::block_on(build_app_state(
                storage,
                secrets,
                &data_dir,
                resource_dir,
            ))
            .expect("build app state");
            app.manage(state);
            // Forward Run Engine broadcast events into the WebView.
            let managed = app.state::<AppState>();
            run_events::spawn(app_handle.clone(), managed.engine.clone());
            deep_link::install(&app_handle);
            scheduler::install(&app_handle);
            menu::install(&app_handle)?;
            tray::install(&app_handle)?;
            push::install(&app_handle);
            updater::install(&app_handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health::health,
            commands::providers::list_providers,
            commands::providers::create_provider,
            commands::providers::update_provider,
            commands::providers::delete_provider,
            commands::providers::provider_presets,
            commands::providers::connect_provider,
            commands::discovery::scan_local_runtimes,
            commands::discovery::probe_local_endpoint,
            commands::discovery::import_local_runtime,
            commands::models::list_models,
            commands::models::verify_tools,
            commands::conversations::list_conversations,
            commands::conversations::create_conversation,
            commands::conversations::send_message,
            commands::conversations::list_messages,
            commands::conversations::rename_conversation,
            commands::conversations::delete_conversation,
            commands::conversations::export_markdown,
            commands::runs::get_run,
            commands::runs::list_runs,
            commands::runs::list_run_events,
            commands::runs::cancel_run,
            commands::runs::resume_run,
            commands::runs::budget_extension,
            commands::runs::list_checkpoints,
            commands::runs::replay_run_events,
            commands::fs::preview_file_diff,
            commands::fs::read_attachment,
            commands::fs::open_external,
            commands::fs::save_text_file,
            commands::fs::save_base64_file,
            commands::approvals::decide_approval,
            commands::tools::list_tools,
            commands::tools::set_permission_rule,
            commands::tools::list_permission_rules,
            commands::tools::list_pending_approvals,
            commands::mcp::list_connections,
            commands::mcp::create_connection,
            commands::mcp::delete_connection,
            commands::mcp::start_oauth,
            commands::mcp::connect,
            commands::mcp::handle_oauth_callback,
            commands::browser::browser_status,
            commands::browser::browser_invoke,
            commands::browser::browser_profiles,
            commands::browser::browser_reset_profile,
            commands::memories::list_memories,
            commands::memories::create_memory,
            commands::memories::update_memory,
            commands::memories::delete_memory,
            commands::memories::list_memory_revisions,
            commands::memories::restore_memory_revision,
            commands::memories::verify_memory,
            commands::memories::search_memories,
            commands::knowledge::list_knowledge,
            commands::knowledge::ingest_knowledge,
            commands::knowledge::import_knowledge_file,
            commands::knowledge::delete_knowledge,
            commands::knowledge::search_knowledge,
            commands::skills::list_skills,
            commands::skills::import_skill,
            commands::skills::export_skill,
            commands::skills::delete_skill,
            commands::skills::list_skill_resources,
            commands::skills::read_skill_resource,
            commands::projects::list_projects,
            commands::projects::create_project,
            commands::projects::update_project,
            commands::projects::delete_project,
            commands::conversations::set_conversation_project,
            commands::workspace::list_dir,
            commands::workspace::read_workspace_file,
            commands::workspace::git_status,
            commands::workspace::git_diff,
            commands::schedules::list_jobs,
            commands::schedules::create_job,
            commands::schedules::set_job_enabled,
            commands::schedules::preview_cron,
            commands::schedules::delete_job,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::agents::list_agents,
            commands::agents::create_agent,
            commands::agents::update_agent,
            commands::agents::delete_agent,
            commands::backups::create_backup,
            commands::backups::restore_backup,
            commands::backups::relaunch_app,
            commands::diagnostics::collect_diagnostics,
            updater::check_for_updates,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MIX-agent Desktop");
}

#[cfg(test)]
mod tests {
    /// The log directory is derived from the bundle identifier without a Tauri
    /// handle, so the constant has to stay in sync with the config file by
    /// hand — this is what catches the drift.
    #[test]
    fn app_identifier_matches_the_tauri_config() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let raw = std::fs::read_to_string(&path).expect("read tauri.conf.json");
        let config: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");
        assert_eq!(
            config.get("identifier").and_then(|v| v.as_str()),
            Some(super::APP_IDENTIFIER),
            "APP_IDENTIFIER drifted from tauri.conf.json"
        );
    }
}
