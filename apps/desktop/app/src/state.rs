use super::*;

pub(crate) async fn parent_model(engine: &RunEngine, run_id: uuid::Uuid) -> anyhow::Result<String> {
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
    pub browser: Arc<webview_browser::WebviewBrowserHost>,
    pub mcp: Arc<McpHost>,
    /// Flips to `true` once the background MCP connection pass has finished.
    ///
    /// Start-up no longer waits for external MCP servers, but the first Run
    /// must not silently lose their tools just because it was sent during the
    /// connect window. `dispatch` waits on this briefly (bounded) before it
    /// hands the request to the engine.
    pub mcp_ready: tokio::sync::watch::Receiver<bool>,
    /// The same runner instance the engine's registry was built with, so a
    /// rebuild after an MCP change keeps the `agent` tool working.
    pub(crate) agent_runner: Arc<EngineAgentRunner>,
    /// Which provider embeds Memory text (memory spec §5). Shared with the
    /// Tool Registry so the `memory` tool and the Memory commands embed with
    /// the same provider, and so a provider/settings change can invalidate the
    /// one cached answer through [`embedding::EmbeddingResolver::invalidate`].
    pub embeddings: Arc<embedding::EmbeddingResolver>,
    /// The same allowlist the Tool Registry uses. The UI browser commands
    /// apply the same gate as agent calls. Behind a lock so saving the settings
    /// form takes effect immediately instead of at the next launch.
    pub egress: Arc<tokio::sync::RwLock<mixd_tools::EgressPolicy>>,
    /// Lifecycle, health and model jobs for local runtimes. One instance for
    /// the whole process so the WebView, the health loop and shutdown all see
    /// the same process ownership.
    pub local_runtime: Arc<local_runtime::LocalRuntimeManager>,
    /// Paths the user has explicitly handed to a native picker (or confirmed
    /// in a dialog). File commands consult this instead of trusting the path
    /// string the WebView sent, so a compromised renderer cannot read or write
    /// arbitrary files without a visible prompt.
    pub grants: Arc<path_grants::PathGrants>,
    /// macOS Spotlight index queue and routing. No-op on other platforms.
    pub spotlight: Arc<spotlight::SpotlightService>,
}

/// Rebuild the engine's Tool Registry from the live parts.
///
/// MCP connect/disconnect changes what the model should be told exists. This
/// used to live in the MCP command module; sharing it keeps a registry rebuild
/// from requiring an app relaunch.
pub(crate) async fn refresh_registry(state: &AppState) {
    let egress = load_egress_policy(state.storage.as_ref()).await;
    // The UI browser commands read this copy, so it has to move with the
    // registry. Saving the egress allowlist used to require a relaunch for the
    // pane while the agent got the new policy on its next registry rebuild.
    *state.egress.write().await = egress.clone();
    let settings = crate::commands::settings::read_settings(state.storage.pool())
        .await
        .unwrap_or_default();
    let mut tools = build_registry(
        &state.storage,
        &state.browser,
        &state.mcp,
        &state.agent_runner,
        &state.embeddings,
        egress,
        settings.discover_user_skills,
    );
    tools.set_mcp_specs(
        mixd_mcp::collect_tool_specs(&state.mcp)
            .await
            .into_iter()
            .map(|spec| mixd_tools::ToolSpec {
                name: spec.name,
                description: spec.description,
                parameters: spec.parameters,
            })
            .collect(),
    );
    state.engine.replace_tools(tools).await;
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
    browser: &Arc<webview_browser::WebviewBrowserHost>,
    mcp: &Arc<McpHost>,
    agent_runner: &Arc<EngineAgentRunner>,
    embeddings: &Arc<embedding::EmbeddingResolver>,
    egress: mixd_tools::EgressPolicy,
    discover_user_skills: bool,
) -> mixd_tools::ToolRegistry {
    // Project skills first: a procedure a repository carries itself is more
    // specific than a personal or stored one with the same name.
    let mut skill_sources: Vec<Arc<dyn mixd_tools::skill::SkillSource>> =
        vec![Arc::new(mixd_core::skills::discovery::ProjectSkillSource)];
    if discover_user_skills {
        skill_sources.push(Arc::new(mixd_core::skills::discovery::UserSkillSource));
    }
    skill_sources.push(Arc::new(DbSkillSource {
        storage: storage.clone(),
    }));
    let mut registry = mixd_tools::ToolRegistry::new(egress)
        .with_mcp(Arc::new(McpBridge { host: mcp.clone() }))
        .with_skills(Arc::new(mixd_tools::skill::CompositeSkillSource::new(
            skill_sources,
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
        }));
    // The native child WebView is always available; there is no install step
    // or executable to resolve anymore.
    registry = registry.with_browser(browser.clone());
    registry
}

pub async fn build_app_state(
    app_handle: tauri::AppHandle,
    storage: Arc<Storage>,
    secrets: Arc<SecretStore>,
    app_data: &std::path::Path,
) -> anyhow::Result<AppState> {
    let build_started = std::time::Instant::now();
    // Old delegate checkouts are removed here rather than on every `agent`
    // call, so a crash does not leak a full copy of the workspace per fan-out.
    // It walks a directory tree, so it runs beside the window rather than in
    // front of it.
    {
        let cleanup_dir = app_data.to_path_buf();
        tauri::async_runtime::spawn(async move {
            let started = std::time::Instant::now();
            cleanup_stale_worktrees(&cleanup_dir).await;
            tracing::info!(
                target: "startup",
                ms = started.elapsed().as_millis() as u64,
                "app_state: stale worktree cleanup"
            );
        });
    }
    // The browser is a native child WebView in the main window. No Chromium
    // child process, download step or screencast is involved.
    let browser = Arc::new(webview_browser::WebviewBrowserHost::new(
        app_handle.clone(),
        app_data,
    ));
    // The managed Chromium tree from older releases is dead weight now that
    // the page is a native WebView. Remove it in the background so an upgrade
    // reclaims the ~200 MB without delaying startup.
    {
        let cleanup_dir = app_data.to_path_buf();
        tauri::async_runtime::spawn(async move {
            let chromium = cleanup_dir.join("chromium");
            if tokio::fs::remove_dir_all(&chromium).await.is_ok() {
                tracing::info!(path = %chromium.display(), "removed the legacy managed Chromium tree");
            }
            let state = cleanup_dir.join("chromium-install-state.json");
            let _ = tokio::fs::remove_file(state).await;
        });
    }

    // Read the settings row once. It was read three times below (browser
    // opt-in, registry, sandbox), which is three SQLite round trips on the
    // path to the first window.
    let settings_started = std::time::Instant::now();
    let settings = match crate::commands::settings::read_settings(storage.pool()).await {
        Ok(settings) => settings,
        Err(err) => {
            tracing::warn!(
                %err,
                "could not read settings; browser/sandbox/skills use their safe defaults"
            );
            Default::default()
        }
    };
    tracing::info!(
        target: "startup",
        ms = settings_started.elapsed().as_millis() as u64,
        "app_state: settings"
    );
    // Apply the persisted browser opt-in before the first run can call the
    // tool. A corrupt/missing settings row simply leaves the safe default off.
    browser.set_allow_local(settings.browser_allow_local).await;

    // Load the MCP *configuration* synchronously (a SQLite read) but connect
    // in the background: `connect_all` talks to external servers and used to
    // hold the first paint behind their timeouts. Until it finishes the
    // registry simply advertises no `mcp__*` tools; a Run that starts in that
    // window waits briefly on `mcp_ready` (see `dispatch`).
    let mcp = Arc::new(McpHost::new());
    let mcp_started = std::time::Instant::now();
    match mixd_mcp::load_configs(storage.as_ref(), Some(secrets.as_ref())).await {
        Ok(configs) => {
            for config in configs {
                mcp.add(config).await;
            }
        }
        Err(err) => {
            tracing::warn!(
                ?err,
                "MCP configuration load failed; continuing without MCP tools"
            );
        }
    }
    tracing::info!(
        target: "startup",
        ms = mcp_started.elapsed().as_millis() as u64,
        "app_state: MCP config load"
    );

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
    let tools = build_registry(
        &storage,
        &browser,
        &mcp,
        &agent_runner,
        &embeddings,
        egress.clone(),
        settings.discover_user_skills,
    );
    let egress = Arc::new(tokio::sync::RwLock::new(egress));

    let engine = Arc::new(RunEngine::with_storage_and_tools(
        storage.clone(),
        secrets.clone(),
        tools,
    ));
    // Connect MCP off the start-up path. The host is shared with the
    // registry, so pushing the specs when the connections settle is enough;
    // no registry rebuild (and no re-read of settings) is needed here.
    let (mcp_ready_tx, mcp_ready_rx) = tokio::sync::watch::channel(false);
    {
        let mcp = mcp.clone();
        let engine = engine.clone();
        tauri::async_runtime::spawn(async move {
            let started = std::time::Instant::now();
            match mcp.connect_all().await {
                Ok(count) => tracing::info!(
                    target: "startup",
                    ms = started.elapsed().as_millis() as u64,
                    tools = count,
                    "MCP servers connected"
                ),
                Err(err) => {
                    tracing::warn!(?err, "MCP connect failed; continuing without MCP tools")
                }
            }
            let specs: Vec<mixd_tools::ToolSpec> = mixd_mcp::collect_tool_specs(&mcp)
                .await
                .into_iter()
                .map(|spec| mixd_tools::ToolSpec {
                    name: spec.name,
                    description: spec.description,
                    parameters: spec.parameters,
                })
                .collect();
            let advertised = specs.len();
            engine.tools.write().await.set_mcp_specs(specs);
            tracing::info!(tools = advertised, "MCP tools advertised");
            // Even a failed connect must release the barrier: a Run should
            // not pay the timeout twice.
            let _ = mcp_ready_tx.send(true);
        });
    }
    // Apply the persisted terminal-sandbox opt-in before the first shell is
    // spawned. A corrupt row leaves the safe default (off) in place.
    {
        // A setting saved on another machine (or before bwrap was removed)
        // must not claim an isolation this host cannot enforce.
        let sandbox = settings.sandbox_terminal && mixd_tools::terminal::os_sandbox_available();
        if settings.sandbox_terminal && !sandbox {
            tracing::warn!(
                reason = mixd_tools::terminal::os_sandbox_unavailable_reason().unwrap_or(""),
                "terminal sandbox setting ignored because the host cannot enforce it"
            );
        }
        engine.set_terminal_sandbox(sandbox).await;
        engine
            .set_terminal_sandbox_network(
                sandbox
                    && settings.sandbox_terminal_network
                    && cfg!(any(target_os = "macos", target_os = "linux")),
            )
            .await;
        // A 15-minute default used to deny prompts while the user was in
        // another app. Apply the stored lifetime before the first turn.
        engine.set_approval_ttl_secs(i64::from(settings.approval_ttl_minutes) * 60);
    }
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
        let engine = engine.clone();
        tauri::async_runtime::spawn(async move {
            // Let the window and the first Run start before spending a
            // provider round trip per batch. Then wait for an idle moment:
            // backfilling every embedding while the user waits for their first
            // answer competed with the Run's own provider calls.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            for _ in 0..120 {
                if engine.active.lock().await.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
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
    let local_runtime =
        local_runtime::LocalRuntimeManager::new(app_handle.clone(), storage.clone());
    // Existing projects were chosen through a dialog on an earlier launch, so
    // their roots are trusted without a fresh prompt. Ad-hoc paths still go
    // through the picker or the native confirmation.
    let grants = Arc::new(path_grants::PathGrants::default());
    seed_project_grants(storage.as_ref(), grants.as_ref()).await;
    let spotlight = spotlight::SpotlightService::new(storage.clone()).await;
    tracing::info!(
        target: "startup",
        ms = build_started.elapsed().as_millis() as u64,
        "app_state: total"
    );
    Ok(AppState {
        storage,
        secrets,
        engine,
        browser,
        mcp,
        mcp_ready: mcp_ready_rx,
        agent_runner,
        embeddings,
        egress,
        local_runtime,
        grants,
        spotlight,
    })
}

/// Seed the grant store with project roots (and their read-only companions)
/// that are already persisted. A corrupt row is skipped: it must not stop the
/// app from launching, and a missing grant only means one extra prompt.
pub(crate) async fn seed_project_grants(storage: &Storage, grants: &path_grants::PathGrants) {
    let rows: Result<Vec<(String,)>, _> =
        sqlx::query_as("SELECT json_extract(data, '$') FROM projects")
            .fetch_all(storage.pool())
            .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(%err, "could not read project roots for the path grant store");
            return;
        }
    };
    for (json,) in rows {
        let Ok(project) = serde_json::from_str::<mixd_storage::records::ProjectData>(&json) else {
            continue;
        };
        grants.grant_permanent(std::path::Path::new(&project.root_path));
        for extra in project.additional_paths {
            grants.grant_permanent(std::path::Path::new(&extra));
        }
    }
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
pub(crate) async fn reconcile_stale_runs(storage: &Storage) {
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
pub(crate) async fn prune_orphan_models(storage: &Storage) {
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

pub(crate) async fn register_providers_from_storage(
    registry: &ProviderRegistry,
    storage: &Storage,
    secrets: &SecretStore,
) -> anyhow::Result<()> {
    type StoredProviderRow = (String, Option<String>, Option<String>, Option<String>, i64);
    let pool = storage.pool();
    let rows: Vec<StoredProviderRow> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.kind') AS kind, json_extract(data, '$.preset') AS preset, \
         json_extract(data, '$.baseUrl') AS base_url, json_extract(data, '$.enabled') AS enabled \
         FROM providers",
    )
    .fetch_all(pool)
    .await?;
    for (id, kind, preset, base_url, enabled) in rows {
        if enabled == 0 {
            continue;
        }
        register_adapter(
            registry,
            secrets,
            &id,
            &kind.unwrap_or_default(),
            preset.as_deref(),
            &base_url.unwrap_or_default(),
            true,
        )
        .await;
    }
    let _ = preset_catalog::PRESETS; // keep the preset table linked
    Ok(())
}
