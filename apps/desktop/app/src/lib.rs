//! MIX-agent Desktop application library.

pub mod commands;
pub mod deep_link;
pub mod diff;
pub mod embedding;
pub mod error;
pub mod http;
pub mod menu;
pub mod push;
pub mod run_events;
pub mod scheduler;
pub mod tray;

use mixd_core::runs::engine::{ProviderRegistry, RunEngine};
use mixd_mcp::McpHost;
use mixd_providers::{
    anthropic::AnthropicProvider, gemini::GeminiProvider, openai::OpenAiProvider,
    preset as preset_catalog, Provider,
};
use mixd_storage::{SecretStore, Storage};
use std::sync::Arc;
use tauri::{Emitter, Manager};

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

/// Bridges the core Memory / Knowledge services into the `memory` Tool.
struct CoreMemoryDispatch {
    storage: Arc<Storage>,
    /// Supplies the query embedding for `recall`, and the vector written for
    /// rows `remember` creates. `resolve` answering `None` — no embeddable
    /// provider — leaves both on the lexical path (memory spec §5.2).
    embeddings: Arc<crate::embedding::EmbeddingResolver>,
}

#[async_trait_impl::async_trait]
impl mixd_tools::memory::MemoryDispatch for CoreMemoryDispatch {
    async fn recall(&self, query: &str, limit: usize) -> anyhow::Result<serde_json::Value> {
        // Hybrid, not lexical: `retrieve_memories_hybrid` embeds the query
        // first and keeps every candidate the lexical path would have kept, so
        // a transport that cannot embed returns the same memories it did
        // before Phase 1 — it just scores them without the `vector` term.
        let source = self.embeddings.resolve().await;
        let hits = mixd_core::retrieval::retrieve_memories_hybrid(
            &self.storage,
            query,
            limit,
            source.as_ref(),
        )
        .await?;
        Ok(serde_json::json!({ "memories": hits }))
    }

    async fn remember(
        &self,
        title: &str,
        content: &str,
        scope: &str,
    ) -> anyhow::Result<serde_json::Value> {
        // A user's「覚えて：…」still means the same thing when it reaches the
        // tool: unwrap the marker so the stored fact is the fact, not the
        // phrase around it (memory spec §6 明示的な記憶).
        //
        // Whether the unwrap *hit* is also what decides the initial lifecycle
        // (spec §3): text that came in through the explicit command is the
        // user's own input and is stored `established`, while the agent
        // reaching for `memory_remember` on its own initiative stores a
        // `candidate` until something confirms it.
        let explicit = mixd_core::memory::explicit_candidate(content);
        let content = explicit
            .clone()
            .unwrap_or_else(|| content.trim().to_string());
        // Spec §8 + §7.2: credentials are rejected, never stored.
        if mixd_core::memory::is_sensitive(&content) {
            return Ok(serde_json::json!({
                "decision": "rejected",
                "id": null,
                "reason": "Memoryには認証情報・トークン・パスワードを保存できません",
            }));
        }
        // One write path with the automatic ones: an identical Memory is
        // reinforced instead of duplicated (spec §7.2's `reinforce` /
        // `duplicate` decisions resolve here).
        let outcome = mixd_core::memory::MemoryService::create_or_reinforce(
            &self.storage,
            mixd_core::memory::MemoryWrite {
                content: &content,
                role: "fact",
                scope,
                source_kind: "agent",
                title: Some(title),
                run_id: None,
                reason: "memory tool",
                candidate_index: 0,
                lifecycle: if explicit.is_some() {
                    mixd_core::memory::EXPLICIT_INITIAL_LIFECYCLE
                } else {
                    mixd_core::memory::AUTO_INITIAL_LIFECYCLE
                },
            },
        )
        .await?;
        let id = match &outcome {
            mixd_core::memory::WriteOutcome::Created(item) => item.id.clone(),
            mixd_core::memory::WriteOutcome::Reinforced(id) => id.clone(),
        };
        // The row is durable now; embedding it afterwards means a provider
        // that cannot embed delays nothing and fails nothing.
        spawn_memory_embed(self.storage.clone(), self.embeddings.clone(), id);
        match outcome {
            mixd_core::memory::WriteOutcome::Created(item) => Ok(serde_json::json!(item)),
            // The reinforced row: same shape, possibly re-read for its
            // post-reinforce strength values.
            mixd_core::memory::WriteOutcome::Reinforced(id) => {
                let item = mixd_core::memory::MemoryService::get(&self.storage, &id).await?;
                Ok(serde_json::json!(item))
            }
        }
    }

    async fn forget(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        mixd_core::memory::MemoryService::delete(&self.storage, id).await?;
        Ok(serde_json::json!({ "deleted": id }))
    }

    async fn search_knowledge(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<serde_json::Value> {
        let hits = mixd_core::retrieval::retrieve_knowledge(&self.storage, query, limit).await?;
        Ok(serde_json::json!({ "knowledge": hits }))
    }
}

/// Re-embed one Memory row without holding up the caller.
///
/// Shared by the `remember` tool and the Memory commands: the row is already
/// durable when this runs, so every outcome — no embeddable provider, a
/// rejected request, a timeout — costs at most a log line, and the row keeps
/// ranking lexically (memory spec §5.2).
pub(crate) fn spawn_memory_embed(
    storage: Arc<Storage>,
    embeddings: Arc<crate::embedding::EmbeddingResolver>,
    id: String,
) {
    tokio::spawn(async move {
        let Some(source) = embeddings.resolve().await else {
            return;
        };
        match mixd_core::memory::MemoryService::embed_item(&storage, &source, &id).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(memory = %id, "embedding unavailable; the memory ranks lexically")
            }
            Err(err) => tracing::warn!(memory = %id, %err, "storing the memory embedding failed"),
        }
    });
}

/// Bridges the core Knowledge services into the `knowledge` Tool.
struct CoreKnowledgeDispatch {
    storage: Arc<Storage>,
}

#[async_trait_impl::async_trait]
impl mixd_tools::knowledge::KnowledgeDispatch for CoreKnowledgeDispatch {
    async fn ingest(&self, filename: &str, text: &str) -> anyhow::Result<serde_json::Value> {
        let doc =
            mixd_core::knowledge::KnowledgeService::ingest(&self.storage, filename, text).await?;
        Ok(
            serde_json::json!({ "doc_id": doc.doc_id, "filename": doc.filename, "chunk_count": doc.chunk_count }),
        )
    }

    async fn delete(&self, doc_id: &str) -> anyhow::Result<serde_json::Value> {
        let removed =
            mixd_core::knowledge::KnowledgeService::delete_doc(&self.storage, doc_id).await?;
        if removed == 0 {
            anyhow::bail!("knowledge document not found: {doc_id}");
        }
        Ok(serde_json::json!({ "deleted": doc_id, "chunks": removed }))
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<serde_json::Value> {
        let hits = mixd_core::retrieval::retrieve_knowledge(&self.storage, query, limit).await?;
        Ok(serde_json::json!({ "knowledge": hits }))
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
    /// Needed to re-hydrate a provider adapter that is not in the registry
    /// yet. Same back-pointer situation as the engine itself.
    secrets: std::sync::OnceLock<Arc<SecretStore>>,
    /// Placeholder model id used only when the parent run predates modelId
    /// persistence. A `std::sync::RwLock` (not `tokio::sync::RwLock`): `attach`
    /// runs inside `tauri::async_runtime::block_on` at startup, i.e. inside a
    /// runtime context where tokio's `blocking_write()` panics with "Cannot
    /// block the current thread from within a runtime" — on macOS that panic
    /// fires inside tao's `did_finish_launching` (nounwind) and aborts the
    /// process. The `delegate` path only clones a short string under the lock,
    /// so holding it briefly without `.await` never stalls the executor.
    model_id: std::sync::RwLock<String>,
}

impl EngineAgentRunner {
    pub(crate) fn attach(
        &self,
        engine: &Arc<RunEngine>,
        secrets: &Arc<SecretStore>,
        model_id: &str,
    ) {
        let _ = self.engine.set(Arc::downgrade(engine));
        let _ = self.secrets.set(secrets.clone());
        // Sync lock: `attach` runs inside `block_on` (runtime context), where
        // tokio's `blocking_write()` panics.
        if let Ok(mut guard) = self.model_id.write() {
            *guard = model_id.to_string();
        }
    }

    fn engine(&self) -> anyhow::Result<Arc<RunEngine>> {
        self.engine
            .get()
            .and_then(|weak| weak.upgrade())
            .ok_or_else(|| {
                anyhow::anyhow!("agent delegation is unavailable before startup completes")
            })
    }

    fn secrets(&self) -> anyhow::Result<&Arc<SecretStore>> {
        self.secrets.get().ok_or_else(|| {
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
        // The parent run is the authority for the model id. The global
        // `model_id` from `attach` used to be read directly, which meant a
        // delegate call in run A could pick up run B's model id when both
        // runs were active at once. Fall back to the placeholder only when
        // the parent run predates modelId being persisted.
        let model_id = match parent_model(&engine, parent.run_id).await {
            Ok(id) => id,
            Err(_) => {
                // Brief sync read (clone a short string, no `.await` while
                // holding the guard), so the executor is never stalled.
                let fallback = self.model_id.read().map(|g| g.clone()).unwrap_or_default();
                if fallback.is_empty() {
                    anyhow::bail!("cannot start a sub-agent: no model id available");
                }
                fallback
            }
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
        // lack of a resolvable model — and so it cannot silently answer through
        // a *different* one, which is what `list().first()` did: the registry
        // sorts by label while the model list sorts by insertion time, so the
        // two could disagree and the sub-agent would answer a different model
        // than the conversation it belongs to.
        let provider_id = resolve_provider_for_model(
            &engine.providers,
            engine.storage.as_ref(),
            self.secrets()?.as_ref(),
            &model_id,
        )
        .await?;
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
            additional_paths: parent.additional_paths.clone(),
            mode: mixd_core::mode::Mode::Thinking,
            model_id,
            system: Some(system),
            // A sub-agent inherits the parent's folder but not its check: it is
            // typically a read-only investigation, and forcing it to run the
            // parent's test suite would double the cost of every delegation.
            verify_command: None,
            format_command: None,
            context_window: 0,
            temperature: None,
            reasoning: None,
            max_tokens: None,
            temporary: false,
            tools: Arc::new(permitted),
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
    /// Which provider embeds Memory text (memory spec §5). Shared with the
    /// Tool Registry so the `memory` tool and the Memory commands embed with
    /// the same provider, and so a provider/settings change can invalidate the
    /// one cached answer through [`embedding::EmbeddingResolver::invalidate`].
    pub embeddings: Arc<embedding::EmbeddingResolver>,
}

/// Assemble the Tool Registry from the live parts.
///
/// Two places build a registry — startup and every MCP connect/disconnect — and
/// they have to produce the same tool list. Rebuilding from scratch used to
/// drop the skill and agent tools (and reset the egress allowlist) because only
/// the startup path attached them, which made an MCP operation silently strip
/// the agent of half its capabilities. The embedding resolver is a parameter
/// for the same reason it is passed in rather than constructed here: both
/// registries have to recall Memory through one shared, cacheable source.
pub(crate) fn build_registry(
    storage: &Arc<Storage>,
    browser: &Arc<mixd_browser::BrowserOperator>,
    mcp: &Arc<McpHost>,
    agent_runner: &Arc<EngineAgentRunner>,
    embeddings: &Arc<embedding::EmbeddingResolver>,
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
        .with_memory(Arc::new(CoreMemoryDispatch {
            storage: storage.clone(),
            embeddings: embeddings.clone(),
        }))
        .with_knowledge(Arc::new(CoreKnowledgeDispatch {
            storage: storage.clone(),
        }))
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
    // One resolver for the whole session: the `memory` tool, the Memory
    // commands and the backfill below all embed through the same cached
    // provider rather than each re-reading the Keychain.
    let embeddings = Arc::new(embedding::EmbeddingResolver::new(
        storage.clone(),
        secrets.clone(),
    ));
    let mut tools = build_registry(&storage, &browser, &mcp, &agent_runner, &embeddings, egress);
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
    agent_runner.attach(&engine, &secrets, "");
    register_providers_from_storage(&engine.providers, &storage, &secrets).await?;
    // Models whose provider row is gone — deleted from the Providers pane, or
    // left behind by a version that never cascaded the delete. They are not
    // just cosmetic: `list_models` offered them, so the Composer let you pick a
    // model no provider could serve. Dropped once at launch so an existing
    // installation cleans itself up.
    prune_orphan_models(storage.as_ref()).await;
    // Runs left in flight by a crash or force-quit can never advance: the
    // cancellation and approval-waiter handles live in memory only.
    reconcile_stale_runs(storage.as_ref()).await;
    // Phase 1 backfill: rows written before embeddings existed, or by a
    // different model, get a vector once per launch. Off the startup path and
    // strictly best effort — with no embeddable provider this task returns
    // immediately, and a batch that fails stops the pass with the remaining
    // rows still ranking lexically (memory spec §5.2).
    {
        let storage = storage.clone();
        let embeddings = embeddings.clone();
        tauri::async_runtime::spawn(async move {
            // Let the window and the first Run start before spending a
            // provider round trip per batch.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let Some(source) = embeddings.resolve().await else {
                return;
            };
            match mixd_core::memory::MemoryService::backfill_embeddings(&storage, &source).await {
                Ok(0) => {}
                Ok(written) => tracing::info!(written, "memory embeddings backfilled"),
                Err(err) => tracing::warn!(
                    %err,
                    "memory embedding backfill failed; the remaining rows rank lexically"
                ),
            }
        });
    }
    Ok(AppState {
        storage,
        secrets,
        engine,
        browser,
        mcp,
        agent_runner,
        embeddings,
    })
}

/// Read the egress allowlist out of `app_settings`.
///
/// `app_settings` holds one JSON document per row, and `SettingsView` has no
/// `rename_all`, so every key on disk is snake_case (`egress_allowlist`). This
/// used to poke at the raw JSON looking for `egressAllowlist`, which never
/// matches, so an allowlist entered in Settings → Network was silently
/// discarded and the tools ran wide-open to every public domain. Going through
/// `read_settings` keeps the two sides from drifting again — a renamed field
/// moves both at once, and a field added without `#[serde(default)]` still
/// degrades to the defaults rather than failing the launch.
///
/// An empty allowlist is unchanged: `EgressPolicy` documents "no allowlist set
/// ⇒ allow public domains, deny private IPs", so only users who actually
/// configured one see a behaviour change.
pub(crate) async fn load_egress_policy(storage: &Storage) -> mixd_tools::EgressPolicy {
    let allowlist = crate::commands::settings::read_settings(storage.pool())
        .await
        .map(|settings| settings.egress_allowlist)
        .unwrap_or_default();
    mixd_tools::EgressPolicy::from_allowlist(allowlist)
}

/// Reconcile Runs left mid-flight by a crash or force-quit.
///
/// The engine keeps its cancellation and approval-waiter handles in memory, so
/// after a restart nothing can advance a `running`, `waiting_approval` or
/// `budget_extension_pending` row: it stays "active" forever, diagnostics
/// counts it as a phantom active Run, and an orphaned approval can never be
/// answered because the waiter that would wake it is gone. Mark them
/// interrupted once at startup and expire their pending prompts.
async fn reconcile_stale_runs(storage: &Storage) {
    let now = chrono::Utc::now().to_rfc3339();
    let runs = sqlx::query(
        "UPDATE runs SET status = 'interrupted', \
         data = json_set(data, '$.status', 'interrupted', '$.updatedAt', ?1) \
         WHERE status IN ('queued','running','waiting_approval','budget_extension_pending')",
    )
    .bind(&now)
    .execute(storage.pool())
    .await;
    match runs {
        Ok(res) if res.rows_affected() > 0 => tracing::warn!(
            rows = res.rows_affected(),
            "marked runs left in flight by a previous session as interrupted"
        ),
        Ok(_) => {}
        Err(err) => tracing::error!(%err, "reconciling stale runs failed"),
    }
    let approvals = sqlx::query("UPDATE approvals SET status = 'expired' WHERE status = 'pending'")
        .execute(storage.pool())
        .await;
    match approvals {
        Ok(res) if res.rows_affected() > 0 => tracing::warn!(
            rows = res.rows_affected(),
            "expired orphaned approvals from a previous session"
        ),
        Ok(_) => {}
        Err(err) => tracing::error!(%err, "expiring orphaned approvals failed"),
    }
}

/// Delete every `models` row whose provider no longer exists.
///
/// A `models` row is only reachable through `data.$.providerId`, and
/// `delete_provider` used to remove the provider row without touching them, so
/// deleting a provider left its whole model list in the table — selectable in
/// the Composer, and guaranteed to fail at request time. The same predicate
/// backs `list_models`, so the rows stay invisible either way; this pass exists
/// to reclaim them on an installation that already accumulated some.
///
/// Best-effort: a failure here must not stop the app from starting.
async fn prune_orphan_models(storage: &Storage) {
    // `COALESCE` is what makes a row with no `providerId` (never possible via
    // `sync_models`, but possible in a hand-edited or restored database) count
    // as an orphan — bare `NULL NOT IN (…)` evaluates to NULL and deletes
    // nothing.
    let result = sqlx::query(
        "DELETE FROM models \
         WHERE COALESCE(json_extract(data, '$.providerId'), '') NOT IN (SELECT id FROM providers)",
    )
    .execute(storage.pool())
    .await;
    match result {
        Ok(res) if res.rows_affected() > 0 => {
            tracing::info!(
                rows = res.rows_affected(),
                "dropped models with no provider"
            )
        }
        Ok(_) => {}
        Err(err) => tracing::error!(%err, "pruning orphaned models failed"),
    }
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
        register_adapter(
            registry,
            secrets,
            &id,
            &kind,
            &base_url.unwrap_or_default(),
            true,
        )
        .await;
    }
    let _ = preset_catalog::PRESETS; // keep the preset table linked
    Ok(())
}

/// Put a stored provider into — or take it out of — the live `ProviderRegistry`.
///
/// The registry is what the Run Engine resolves a model's `provider:` prefix
/// against, and it is the *only* thing `execute` consults
/// (`runs::engine`: `providers.get(&request.provider_id)`). Boot hydration is
/// not enough on its own: a provider the user adds, edits, disables or fetches
/// models for during the session used to stay absent from the registry until
/// the next launch, which surfaced as `no provider configured` on the very
/// message they had just enabled a model for.
///
/// A disabled provider is removed rather than registered, so it cannot be
/// reached through any model id either.
pub async fn register_adapter(
    registry: &ProviderRegistry,
    secrets: &SecretStore,
    id: &str,
    kind: &str,
    base_url: &str,
    enabled: bool,
) {
    if !enabled {
        registry.unregister(id).await;
        return;
    }
    let api_key = secrets
        .get_string(&format!("provider/{id}"), "api_key")
        .await
        .ok();
    registry
        .register(
            id.to_string(),
            build_provider(kind, base_url.to_string(), api_key),
        )
        .await;
}

/// The provider that will actually serve `model_id`.
///
/// Model ids are stored as `<provider_id>:<model>`, so the prefix names the
/// provider exactly. When the registry does not have it — a provider added
/// before a fix shipped, one whose adapter was built from stale credentials,
/// or one that was disabled and re-enabled — the row is read back from storage
/// and the adapter is built on the spot. Without that step the only way out was
/// "no provider configured" plus a restart, which is what made a correctly
/// configured provider look broken.
///
/// Falls back to the first registered provider only for an id that carries no
/// resolvable prefix (a bare model name). A prefixed id whose provider is gone
/// (deleted, disabled, or never existed) is an error — silently serving it
/// from whatever provider sorts first answers through the wrong model.
pub async fn resolve_provider_for_model(
    registry: &ProviderRegistry,
    storage: &Storage,
    secrets: &SecretStore,
    model_id: &str,
) -> anyhow::Result<String> {
    if let Some((prefix, _)) = model_id.split_once(':') {
        if registry.get(prefix).await.is_some() {
            return Ok(prefix.to_string());
        }
        if let Some((kind, base_url, enabled)) = provider_row(storage, prefix).await? {
            if enabled {
                register_adapter(registry, secrets, prefix, &kind, &base_url, true).await;
                return Ok(prefix.to_string());
            }
            anyhow::bail!(
                "provider not found for model: {model_id} (provider `{prefix}` is disabled)"
            );
        }
        anyhow::bail!("provider not found for model: {model_id} (unknown provider `{prefix}`)");
    }
    registry
        .list()
        .await
        .first()
        .map(|h| h.id.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "プロバイダーが未設定です。設定 → プロバイダー から追加してください。 \
                 (no provider configured for model: {model_id})"
            )
        })
}

/// `(kind, base_url, enabled)` for one stored provider, `None` when the id is
/// unknown.
async fn provider_row(
    storage: &Storage,
    id: &str,
) -> anyhow::Result<Option<(String, String, bool)>> {
    let row: Option<(Option<String>, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT json_extract(data, '$.kind'), json_extract(data, '$.baseUrl'), \
         json_extract(data, '$.enabled') FROM providers WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(storage.pool())
    .await?;
    Ok(row.map(|(kind, base_url, enabled)| {
        (
            kind.unwrap_or_default(),
            base_url.unwrap_or_default(),
            // `enabled` is absent from rows written before the field existed;
            // those were all enabled.
            enabled.unwrap_or(1) != 0,
        )
    }))
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
///
/// Single-sourced: this aliases the Keychain service name in `mixd_storage`
/// (the dependency runs app → storage, so storage cannot alias us). The
/// three-way test below asserts the config file, this constant, and the
/// Keychain service name all agree.
const APP_IDENTIFIER: &str = mixd_storage::SERVICE;

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

    let result = tauri::Builder::default()
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
        // Signed, verified updates. The WebView reaches this only through
        // `commands::update`, so no `updater:*` capability is granted.
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
            // Startup failures are logged and returned rather than panicked:
            // with `panic = "abort"` an `.expect` here killed the process with
            // no diagnostic a user could act on.
            //
            // Writability pre-check: `Storage::open` also creates the dir, but
            // surfacing a permission failure here names the fix (ownership /
            // ACLs / read-only volume) before SQLite reports a bare I/O error.
            // `block_on` has no timeout on Tauri's runtime, so a wedged open
            // (stale lock, unresponsive volume) would hang the splash screen;
            // the error text below therefore names the remedies (permissions,
            // stale lock/WAL cleanup, restart) instead of just the path.
            if let Err(err) = std::fs::create_dir_all(&data_dir) {
                tracing::error!(%err, dir = %data_dir.display(), "data directory is not writable");
                return Err(Box::<dyn std::error::Error>::from(format!(
                    "could not prepare the data directory at {}: {err}. \
                     Check that the directory is writable (folder permissions / ownership, \
                     read-only volume, full disk). If the app previously crashed, a stale \
                     lock or -wal/-shm sidecar may also wedge startup — removing them and \
                     restarting usually clears it.",
                    data_dir.display()
                )));
            }
            let storage =
                match tauri::async_runtime::block_on(mixd_storage::Storage::open(&data_dir)) {
                    Ok(storage) => std::sync::Arc::new(storage),
                    Err(err) => {
                        tracing::error!(%err, dir = %data_dir.display(), "could not open storage");
                        return Err(Box::<dyn std::error::Error>::from(format!(
                            "could not open storage at {}: {err}. \
                             Check that the directory is writable and not locked by another \
                             instance (a stale mix-agent.db-wal/-shm from a crash can wedge \
                             startup — quit all instances, remove the sidecars, and restart). \
                             If the database itself is corrupt, restore from a backup.",
                            data_dir.display()
                        )));
                    }
                };
            let secrets = std::sync::Arc::new(mixd_storage::SecretStore::open(&data_dir));
            // Tauri's resolved resource dir is where the bundled Chromium was
            // unpacked; guessing it from app_data breaks on installed builds.
            // Under cargo/tauri dev that dir is `target/debug`, whose Chromium
            // mirror has lost the macOS framework symlinks, so debug builds read
            // the pristine payload next to this crate instead. `env!` is
            // resolved at compile time and cfg-gated out of release builds, so
            // no local path is baked into a shipped binary.
            let manifest_resources =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
            let resource_dir = mixd_browser::resolve_resource_dir(
                app.path().resource_dir().ok().as_deref(),
                Some(manifest_resources.as_path()),
            );
            let state = match tauri::async_runtime::block_on(build_app_state(
                storage,
                secrets,
                &data_dir,
                resource_dir,
            )) {
                Ok(state) => state,
                Err(err) => {
                    tracing::error!(%err, "could not build the application state");
                    return Err(Box::<dyn std::error::Error>::from(format!(
                        "could not build the application state: {err}. \
                         This usually means a provider row or MCP config failed to load — \
                         check the log file in the data directory, fix or remove the \
                         offending row, and restart the app."
                    )));
                }
            };
            app.manage(state);
            // Forward Run Engine broadcast events into the WebView.
            let managed = app.state::<AppState>();
            run_events::spawn(app_handle.clone(), managed.engine.clone());
            deep_link::install(&app_handle);
            scheduler::install(&app_handle);
            menu::install(&app_handle)?;
            tray::install(&app_handle)?;
            push::install(&app_handle);
            Ok(())
        })
        // Closing the window keeps a live run alive in the tray. Before this,
        // the default "exit when the last window closes" killed streaming runs
        // and made the tray unreachable, contradicting its own documentation.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                let _ = window.app_handle().emit("app://hidden-to-tray", ());
            }
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
            commands::models::set_model_auto,
            commands::auto::resolve_auto,
            commands::auto::set_message_feedback,
            commands::conversations::list_conversations,
            commands::conversations::create_conversation,
            commands::conversations::send_message,
            commands::conversations::edit_message,
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
            commands::browser::browser_artifact,
            commands::memories::list_memories,
            commands::memories::create_memory,
            commands::memories::update_memory,
            commands::memories::delete_memory,
            commands::memories::list_memory_revisions,
            commands::memories::restore_memory_revision,
            commands::memories::verify_memory,
            commands::memories::search_memories,
            commands::memories::memory_associations,
            commands::memories::resolve_conflict,
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
            commands::workspace::find_workspace_files,
            commands::workspace::read_workspace_file,
            commands::workspace::git_status,
            commands::workspace::git_diff,
            commands::workspace::git_restore,
            commands::workspace::git_stage,
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
            commands::diagnostics::get_recent_logs,
            commands::diagnostics::export_diagnostics,
            commands::update::app_version,
            commands::update::check_for_update,
            commands::update::install_update,
        ])
        .build(tauri::generate_context!());

    let app = match result {
        Ok(app) => app,
        Err(err) => {
            tracing::error!(%err, "MIX-agent Desktop exited with an error");
            std::process::exit(1);
        }
    };
    app.run(|app_handle, event| {
        // Dock icon / activation on macOS must bring the hidden window back.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } = event
        {
            if !has_visible_windows {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app_handle, event);
    });
}

#[cfg(test)]
mod tests {
    /// The log directory is derived from the bundle identifier without a Tauri
    /// handle, so the constant has to stay in sync with the config file by
    /// hand — this is what catches the drift. The Keychain service name is the
    /// third party: it aliases the same string, so all three are asserted.
    #[test]
    fn app_identifier_matches_the_tauri_config() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let raw = std::fs::read_to_string(&path).expect("read tauri.conf.json");
        let config: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");
        let identifier = config
            .get("identifier")
            .and_then(|v| v.as_str())
            .expect("tauri.conf.json has an identifier");
        assert_eq!(
            identifier,
            super::APP_IDENTIFIER,
            "APP_IDENTIFIER drifted from tauri.conf.json"
        );
        assert_eq!(
            identifier,
            mixd_storage::SERVICE,
            "Keychain SERVICE drifted from tauri.conf.json"
        );
    }

    /// Apple Silicon refuses to launch a completely unsigned `.app` copied out
    /// of a downloaded DMG and reports it as damaged. Keep the ad-hoc signing
    /// identity in the config so a release build can never regress to unsigned
    /// again; Developer ID signing/notarization remains opt-in.
    #[test]
    fn macos_bundle_keeps_the_ad_hoc_signing_identity() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let raw = std::fs::read_to_string(&path).expect("read tauri.conf.json");
        let config: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");
        let identity = config
            .pointer("/bundle/macOS/signingIdentity")
            .and_then(|v| v.as_str());
        assert_eq!(
            identity,
            Some("-"),
            "bundle.macOS.signingIdentity must stay \"-\" (ad-hoc) so macOS can launch the app"
        );
    }

    /// Extract `version = "x.y.z"` from exactly one TOML section.
    ///
    /// A naive scan for the first `version` line after `[workspace.package]`
    /// never notices the next section header, so a `version` key in a later
    /// section (or a commented-out line) could be picked up silently. This
    /// parser tracks the current section, stops at the next header, skips
    /// comments/blank lines, and requires exactly one quoted `version` value
    /// inside the section.
    fn toml_section_version(toml: &str, section: &str) -> String {
        let mut in_section = false;
        let mut found: Option<String> = None;
        for raw_line in toml.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                // A header ends at the first `]`; anything after it (trailing
                // comment) is ignored for section tracking.
                let header = match line.find(']') {
                    Some(idx) => line[..=idx].trim(),
                    None => line,
                };
                in_section = header == section;
                continue;
            }
            if !in_section {
                continue;
            }
            // Strip a trailing comment (`version = "0.1.0" # comment`).
            let code = match line.find('#') {
                Some(idx) => line[..idx].trim_end(),
                None => line,
            };
            let Some(rest) = code.strip_prefix("version") else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                continue;
            };
            let value = rest.trim();
            assert!(
                value.len() >= 2 && value.starts_with('"') && value.ends_with('"'),
                "version in {section} must be a quoted string, got: {value}"
            );
            assert!(found.is_none(), "duplicate version key in {section}");
            found = Some(value[1..value.len() - 1].to_string());
        }
        found.unwrap_or_else(|| panic!("no version key in {section}"))
    }

    /// `version` is declared in 4 places (workspace Cargo.toml, app Cargo via
    /// workspace, tauri.conf.json, package.json + ui/package.json) and they
    /// must ship the same number — this is what catches the drift.
    #[test]
    fn app_version_matches_all_manifests() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_toml = std::fs::read_to_string(manifest_dir.join("..").join("Cargo.toml"))
            .expect("read workspace Cargo.toml");
        let cargo_version = toml_section_version(&workspace_toml, "[workspace.package]");
        let tauri_raw = std::fs::read_to_string(manifest_dir.join("tauri.conf.json"))
            .expect("read tauri.conf.json");
        let tauri: serde_json::Value =
            serde_json::from_str(&tauri_raw).expect("parse tauri.conf.json");
        let tauri_version = tauri
            .get("version")
            .and_then(|v| v.as_str())
            .expect("tauri version");
        assert_eq!(
            env!("CARGO_PKG_VERSION"),
            cargo_version.as_str(),
            "CARGO_PKG_VERSION drifted from workspace Cargo.toml"
        );
        assert_eq!(
            tauri_version,
            cargo_version.as_str(),
            "tauri.conf.json version drifted from workspace Cargo.toml"
        );
        for rel in ["../package.json", "../ui/package.json"] {
            let raw = std::fs::read_to_string(manifest_dir.join(rel)).expect("read package.json");
            let pkg: serde_json::Value = serde_json::from_str(&raw).expect("parse package.json");
            let v = pkg
                .get("version")
                .and_then(|v| v.as_str())
                .expect("pkg version");
            assert_eq!(v, cargo_version.as_str(), "version drift in {rel}");
        }
    }

    async fn temp_storage() -> mixd_storage::Storage {
        let dir = std::env::temp_dir().join(format!("mixd-egress-test-{}", uuid::Uuid::new_v4()));
        mixd_storage::Storage::open(&dir)
            .await
            .expect("open temp storage")
    }

    /// The allowlist entered in Settings → Network has to survive the round trip
    /// into the policy the tools actually enforce. It is stored snake_case
    /// (`SettingsView` has no `rename_all`), and the loader once looked for
    /// `egressAllowlist` instead — so every configured allowlist was silently
    /// dropped and `web_fetch` ran wide-open to every public domain.
    #[tokio::test]
    async fn the_configured_egress_allowlist_reaches_the_policy() {
        let storage = temp_storage().await;
        sqlx::query("INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?)")
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(r#"{"egress_allowlist":["api.example.com","*.internal.test"]}"#)
            .execute(storage.pool())
            .await
            .expect("insert settings row");

        let policy = super::load_egress_policy(&storage).await;
        assert_eq!(policy.allowlist, ["api.example.com", "*.internal.test"]);
        assert!(policy.enforce("https://api.example.com/v1").is_ok());
        assert!(policy.enforce("https://elsewhere.test/v1").is_err());
    }

    /// The wide-open default is a documented behaviour, not an accident: an
    /// empty allowlist allows public hosts and still refuses private IPs.
    #[tokio::test]
    async fn an_unconfigured_allowlist_stays_wide_open_for_public_hosts() {
        let storage = temp_storage().await;
        let policy = super::load_egress_policy(&storage).await;
        assert!(policy.allowlist.is_empty());
        assert!(policy.enforce("https://example.com/").is_ok());
        assert!(policy.enforce("https://127.0.0.1/").is_err());
    }

    /// A row written by an older build must not fail the launch: the tools fall
    /// back to the defaults rather than the app refusing to start.
    #[tokio::test]
    async fn an_unreadable_settings_row_still_yields_a_usable_policy() {
        let storage = temp_storage().await;
        sqlx::query(
            "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, 'not json')",
        )
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .execute(storage.pool())
        .await
        .expect("insert garbage row");

        let policy = super::load_egress_policy(&storage).await;
        assert!(policy.allowlist.is_empty());
    }
}
