use super::*;

/// Bridges the MCP host into the Tool Registry's `mcp__*` dispatcher.
pub(crate) struct McpBridge {
    pub(crate) host: Arc<McpHost>,
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
pub(crate) struct CoreMemoryDispatch {
    pub(crate) storage: Arc<Storage>,
    /// Supplies the query embedding for `recall`, and the vector written for
    /// rows `remember` creates. `resolve` answering `None` — no embeddable
    /// provider — leaves both on the lexical path (memory spec §5.2).
    pub(crate) embeddings: Arc<crate::embedding::EmbeddingResolver>,
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
pub(crate) struct CoreKnowledgeDispatch {
    pub(crate) storage: Arc<Storage>,
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
pub(crate) struct DbSkillSource {
    pub(crate) storage: Arc<Storage>,
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
                    resources: skill
                        .resources
                        .iter()
                        .map(|resource| resource.path.clone())
                        .collect(),
                    name: skill.name,
                    description: skill.description,
                    body_markdown: skill.body_markdown,
                    license: skill.license,
                    compatibility: skill.compatibility,
                    metadata: skill.metadata,
                    allowed_tools: skill.allowed_tools,
                },
            )
            .collect())
    }

    async fn resource(
        &self,
        _root: Option<&str>,
        skill: &str,
        path: &str,
    ) -> anyhow::Result<Option<mixd_tools::skill::SkillResourceData>> {
        let Some(skill) =
            mixd_core::skills::SkillService::get(self.storage.as_ref(), skill).await?
        else {
            return Ok(None);
        };
        Ok(skill
            .resources
            .into_iter()
            .find(|resource| resource.path == path)
            .map(|resource| mixd_tools::skill::SkillResourceData {
                path: resource.path,
                content: resource.content,
                binary: resource.binary,
                size: resource.size,
            }))
    }
}

/// Serves the saved agent profiles to the `agent` Tool.
pub(crate) struct DbAgentSource {
    pub(crate) storage: Arc<Storage>,
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
                    model_id: value
                        .get("modelId")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string),
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
        // A profile may name its own model (a cheap read-only explorer vs. the
        // parent's coding model). An unknown id falls back to the parent's
        // provider resolution error rather than silently using another model.
        let model_id = profile
            .model_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or(model_id);
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
            keep_alive: None,
            temporary: false,
            tools: Arc::new(permitted),
            history: Vec::new(),
            // A blank user message keeps the persisted transcript distinguishable
            // from the parent's; the real instruction is the system prompt.
            user_message: task.to_string(),
            resume_from: None,
            auto_fallbacks: Vec::new(),
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
            ..Default::default()
        })
    }

    /// Run the delegate in a detached git worktree.
    ///
    /// Two fan-out tasks editing the same file used to race in the parent
    /// workspace; the last writer won and the diff was unreviewable. A worktree
    /// gives each delegate a private checkout and the parent gets a patch it can
    /// apply deliberately.
    async fn delegate_isolated(
        &self,
        parent: &mixd_tools::ToolContext,
        profile: &mixd_tools::agent::AgentProfile,
        task: &str,
    ) -> anyhow::Result<mixd_tools::agent::DelegateResult> {
        let Some(root) = parent.root_path.as_deref() else {
            let mut fallback = self.delegate(parent, profile, task).await?;
            fallback
                .text
                .push_str("\n\n(worktree isolation unavailable: the run has no workspace root)");
            return Ok(fallback);
        };
        let Some((worktree, base)) = prepare_delegate_worktree(root, parent.run_id).await else {
            let mut fallback = self.delegate(parent, profile, task).await?;
            fallback.text.push_str(
                "\n\n(worktree isolation unavailable: the workspace is not a git repository \
                 or git worktree failed; changes went to the parent workspace)",
            );
            return Ok(fallback);
        };

        let mut scoped = parent.clone();
        scoped.root_path = Some(worktree.to_string_lossy().into_owned());
        let outcome = self.delegate(&scoped, profile, task).await;
        let (diff, changed_files) = collect_worktree_diff(&worktree, &base).await;
        cleanup_delegate_worktree(root, &worktree).await;

        let mut result = outcome?;
        // The worktree was temporary: `cleanup_delegate_worktree` has already
        // removed it, so returning its path would point at a directory that no
        // longer exists. The diff and changed-file list are the durable result.
        result.diff = diff;
        result.changed_files = changed_files;
        Ok(result)
    }
}
