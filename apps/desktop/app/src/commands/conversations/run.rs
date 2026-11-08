use super::*;

pub async fn dispatch_run(
    state: &AppState,
    input: SendMessageInput,
) -> Result<SendMessageOutput, String> {
    dispatch(state, input, None).await
}

/// The turn an edit branches from, identified the way the transcript orders
/// messages: by `created_at` with `rowid` breaking ties.
pub struct TruncateTail {
    pub(crate) created_at: String,
    pub(crate) rowid: i64,
}

/// Start a Run, optionally replacing an existing tail first.
///
/// The truncation, the new Run row and the user turn are written in a single
/// transaction, so an edit either replaces the old tail or leaves the
/// conversation untouched. The engine is spawned only after that commit, which
/// is what keeps a failure while preparing the Run from stranding a transcript
/// that has already been deleted.
pub async fn dispatch(
    state: &AppState,
    input: SendMessageInput,
    truncate: Option<TruncateTail>,
) -> Result<SendMessageOutput, String> {
    validate_send_input(&input)?;
    // MCP connects in the background now, so the first Run may arrive while
    // the servers are still starting. Wait for that pass, but only briefly:
    // a slow or unreachable MCP server must not make the app feel hung.
    {
        let mut ready = state.mcp_ready.clone();
        if !*ready.borrow() {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                ready.wait_for(|connected| *connected),
            )
            .await;
        }
    }
    let pool = state.storage.pool();
    let row: Option<ConversationDispatchRow> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.mode'), json_extract(data, '$.modelId'), \
          json_extract(data, '$.title'), json_extract(data, '$.updatedAt'), \
          json_extract(data, '$.workspaceId') \
          FROM conversations WHERE id = ?",
    )
    .bind(&input.conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    let (_, stored_mode, stored_model, _, _, stored_workspace) =
        row.ok_or_else(|| "conversation not found".to_string())?;

    // Explicit memory intake (memory spec §6 明示的な記憶): a「覚えて：…」or
    //「…を忘れて」message is saved / deleted synchronously here, before the Run
    // starts — waiting for post-response extraction would be the whole point
    // of saying it out loud. A temporary turn skips it, matching the Docker
    // edition's `_handle_explicit_memory_requests` (no trace, no memory).
    if !input.temporary_mode {
        match mixd_core::memory::handle_explicit(&state.storage, &input.content).await {
            Ok(outcome) => {
                if let Some(id) = outcome.created {
                    // The row is durable now; embedding it in the background
                    // means a provider without an embeddings endpoint never
                    // delays the turn that asked for it (spec §5.2).
                    crate::spawn_memory_embed(state.storage.clone(), state.embeddings.clone(), id);
                }
                if !outcome.forgotten.is_empty() {
                    tracing::info!(
                        forgotten = outcome.forgotten.len(),
                        "explicit memory forget applied"
                    );
                }
            }
            // The turn matters more than the intake: a storage hiccup while
            // saving an explicit memory must not block the message itself.
            Err(err) => tracing::warn!(%err, "explicit memory intake failed"),
        }
    }

    // A mode passed by the caller (the composer's mode selector) overrides the
    // mode stored at conversation creation; without this the selector was
    // silently ignored for existing conversations.
    let mode_str = input
        .mode
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| stored_mode.clone());
    let mode: mixd_core::Mode = mode_str.parse().map_err(|err| {
        tracing::warn!(%err, "dispatch: rejecting unknown mode");
        "invalid mode".to_string()
    })?;
    if mode_str != stored_mode {
        mixd_storage::log_write(
            sqlx::query("UPDATE conversations SET data = json_set(data, '$.mode', ?) WHERE id = ?")
                .bind(&mode_str)
                .bind(&input.conversation_id)
                .execute(pool)
                .await,
            "persist conversation mode",
        );
    }
    let requested_model_id = input.model_id.clone().unwrap_or(stored_model);
    // Docker parity: `auto` resolves to the best candidate per request. The
    // resolved id serves the turn; `requested_model_id` is kept so feedback
    // and history know this was an Auto turn.
    let is_auto = requested_model_id == "auto" || requested_model_id.is_empty();
    // Prefer an explicitly requested workspace, then the one the conversation
    // already owns, and only mint a fresh id for a conversation created before
    // workspaces were persisted. Regenerating it on every message (the old
    // behaviour) silently gave the agent a new browser profile, terminal
    // session and permission scope each turn.
    let stored_workspace = stored_workspace.and_then(|w| Uuid::parse_str(&w).ok());
    let requested_workspace = input
        .workspace_id
        .clone()
        .and_then(|w| Uuid::parse_str(&w).ok());
    let workspace_id = requested_workspace
        .or(stored_workspace)
        .unwrap_or_else(Uuid::new_v4);
    if stored_workspace != Some(workspace_id) {
        // Persist it back so the browser pane and later Runs resolve the same
        // profile.
        mixd_storage::log_write(
            sqlx::query(
                "UPDATE conversations SET data = json_set(data, '$.workspaceId', ?) WHERE id = ?",
            )
            .bind(workspace_id.to_string())
            .bind(&input.conversation_id)
            .execute(pool)
            .await,
            "persist conversation workspace",
        );
    }

    // Project context: instructions / sources / AGENTS.md → system prompt,
    // and the project folder → tool workspace root. Read-only and independent
    // of the new turn, so resolve it before opening the write transaction.
    let mut project =
        crate::commands::projects::load_project_runtime(pool, &input.conversation_id).await?;
    // Captured before the scratch fallback and before `project.root_path` is
    // moved into the RunRequest below. A scratch folder is not a Git
    // repository, so it must not become the rollback baseline.
    let baseline_root = project.root_path.clone();
    if project.root_path.is_none() {
        let scratch = projectless_scratch_root(state, &input.conversation_id)?;
        project.system = Some(match project.system.take() {
            Some(mut existing) => {
                existing.push_str("\n\n");
                existing.push_str(&projectless_workspace_note(&scratch));
                existing
            }
            None => projectless_workspace_note(&scratch),
        });
        project.root_path = Some(scratch);
    }

    let tools = {
        let all = build_tool_specs(state, mode).await;
        if !input.temporary_mode {
            all
        } else if !input.allow_tools {
            // A temporary turn is private by default: no tool may leave a trace
            // on disk, the network or Memory.
            Arc::new(Vec::new())
        } else {
            // Even with tools allowed, Memory and Skill tools are excluded: they
            // exist to read and write durable state, which is what the mode is
            // opting out of.
            Arc::new(
                all.iter()
                    .filter(|spec| {
                        spec.name != "memory"
                            && spec.name != "skill"
                            && spec.name != "skill_resource"
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        }
    };
    let reasoning = match input.reasoning.as_deref() {
        Some("low") => Some(ReasoningControl::Low),
        Some("medium") => Some(ReasoningControl::Medium),
        Some("high") => Some(ReasoningControl::High),
        Some("xhigh") => Some(ReasoningControl::Xhigh),
        _ => None,
    };

    let run_id = RunId::new_v4();
    let now = Utc::now();
    // Persist the mode's full budget (including the token ceiling) rather
    // than a hand-written subset: a missing `tokenLimit` silently falls back
    // to the load-time base, which downgrades agent runs when the base is the
    // 12-step default instead of the mode's own budget.
    let budget = mixd_core::runs::budget::Budget::for_mode(mode).to_json();
    let request_key = limits::request_key(
        &input.conversation_id,
        now.timestamp_millis(),
        &run_id.to_string(),
    );

    // Resolve the serving model. `auto` picks the best candidate per request;
    // anything else is served as-is.
    let has_image = input
        .attachments
        .iter()
        .any(|a| a.mime_type.starts_with("image/") || a.kind == "image");
    let tools_required = mode == mixd_core::Mode::Agent && !tools.is_empty();
    let (model_id, provider_id, auto_selection, auto_fallbacks): (
        String,
        String,
        Option<mixd_core::auto::AutoSelection>,
        Vec<AutoFallback>,
    ) = if is_auto {
        let prior_contents: Vec<String> = sqlx::query_scalar::<_, Option<String>>(
            "SELECT json_extract(data, '$.content') FROM messages \
              WHERE conversation_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(&input.conversation_id)
        .bind(limits::AUTO_CONTEXT_LIMIT)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect();
        let mut context_parts: Vec<String> = prior_contents.into_iter().rev().collect();
        context_parts.push(input.content.clone());
        // System instructions and tool schemas are part of the prompt even
        // though they are not transcript messages. Leaving them out made Auto
        // pick a model that then failed on the first turn because its window
        // was smaller than the actual request.
        if let Some(system) = project.system.as_deref() {
            context_parts.push(system.to_string());
        }
        if let Ok(tools_json) = serde_json::to_string(tools.as_ref()) {
            context_parts.push(tools_json);
        }
        let attachment_tokens = input
            .attachments
            .iter()
            .map(attachment_token_estimate)
            .sum::<u64>();
        let resolved = crate::commands::auto::resolve_auto_model(
            pool,
            &input.content,
            mode.as_str(),
            tools_required,
            has_image,
            &context_parts,
            attachment_tokens,
            limits::auto_reserved_output_tokens(mode),
            &request_key,
            None,
        )
        .await?;
        let provider = resolve_provider_id(state, &resolved.model_id).await?;
        debug_assert_eq!(provider, resolved.provider_id);
        // Resolve the fallback chain now, while the candidate list and
        // reliability signals are already warm. The engine only switches when
        // a provider fails before emitting any content; it never guesses a
        // model on its own.
        let attempt_limit = crate::commands::settings::read_settings(pool)
            .await
            .ok()
            .map(|settings| settings.auto_fallback_attempts)
            .unwrap_or(1)
            .min(2);
        let mut excluded = vec![resolved.model_id.clone()];
        let mut fallbacks: Vec<AutoFallback> = Vec::new();
        let budget_cap = crate::commands::settings::read_settings(pool)
            .await
            .ok()
            .and_then(|settings| settings.monthly_budget_usd)
            .filter(|cap| *cap > 0.0);
        let cloud_blocked = match budget_cap {
            Some(cap) => crate::commands::settings::monthly_spend_usd(pool)
                .await
                .map(|spent| spent >= cap)
                .unwrap_or(false),
            None => false,
        };
        for index in 0..attempt_limit {
            let next = match crate::commands::auto::resolve_auto_model_excluding(
                pool,
                &input.content,
                mode.as_str(),
                tools_required,
                has_image,
                &context_parts,
                attachment_tokens,
                limits::auto_reserved_output_tokens(mode),
                &format!("{request_key}:fallback:{index}"),
                None,
                &excluded,
                Some(&resolved.provider_id),
            )
            .await
            {
                Ok(next) => next,
                Err(_) => break,
            };
            excluded.push(next.model_id.clone());
            // A local primary model must not silently hand the turn to a paid
            // provider after the monthly cap is reached.
            if cloud_blocked {
                let locality =
                    crate::commands::providers::provider_locality(pool, &next.provider_id).await;
                if !locality.is_local {
                    continue;
                }
            }
            fallbacks.push(AutoFallback {
                model_id: next.model_id,
                provider_id: next.provider_id,
                context_window: next.context_window,
                keep_alive: None,
            });
        }
        (
            resolved.model_id,
            provider,
            Some(resolved.selection),
            fallbacks,
        )
    } else {
        let provider_id = resolve_provider_id(state, &requested_model_id).await?;
        (requested_model_id.clone(), provider_id, None, Vec::new())
    };

    // Hard monthly budget. Checked after the provider is resolved so a local
    // model (estimated cost zero) keeps working even after a cloud budget is
    // exhausted. The value is an estimate and therefore a guardrail, not a
    // billing statement.
    if let Ok(settings) = crate::commands::settings::read_settings(pool).await {
        if let Some(cap) = settings.monthly_budget_usd.filter(|cap| *cap > 0.0) {
            let locality = crate::commands::providers::provider_locality(pool, &provider_id).await;
            if !locality.is_local {
                let spent = crate::commands::settings::monthly_spend_usd(pool).await?;
                if spent >= cap {
                    return Err(format!(
                        "月次予算 ${cap:.2} に達しています（今月の推定使用額 ${spent:.2}）。設定 ▸ 一般 で上限を変更してください。"
                    ));
                }
            }
        }
    }

    // The truncation, the Run row and the user turn share one transaction: an
    // edit either replaces the old tail with a new Run, or leaves the
    // conversation exactly as it was. Splitting them (truncate, commit, then
    // start the Run) left the transcript deleted with nothing to replace it
    // whenever Run creation failed.
    let mut tx = pool.begin().await.map_err(crate::error::cmd_err)?;
    if truncate.is_some() {
        // The engine appends assistant turns to a transcript this edit is about
        // to rewrite, so an in-flight Run must finish (or be cancelled) first.
        // Checking inside the transaction closes the window between the check
        // and the delete that would let a Run start against a deleted tail.
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM runs WHERE conversation_id = ? AND status IN \
             ('queued','running','waiting_approval','budget_extension_pending')",
        )
        .bind(&input.conversation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
        if active > 0 {
            return Err("実行中はメッセージを編集できません".to_string());
        }
    }
    // `request_key` is UNIQUE: a retry that reuses a key must not create a
    // second Run. The key embeds a fresh `run_id`, so collisions are only
    // possible on explicit retry — check before the INSERT to fail loudly
    // instead of surfacing a bare UNIQUE-constraint error.
    let duplicate: Option<String> = sqlx::query_scalar("SELECT id FROM runs WHERE request_key = ?")
        .bind(&request_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    if duplicate.is_some() {
        return Err("duplicate request (request_key already exists)".to_string());
    }
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, ?, ?, 'queued', ?)",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(
        serde_json::json!({
            "mode": mode.as_str(),
            "modelId": model_id,
            // Which provider served this Run — the memory extraction job
            // (memory spec §6) later replays the call through the same one.
            "providerId": provider_id,
            "requestedModelId": requested_model_id,
            "autoSelection": auto_selection,
            "workspaceId": workspace_id.to_string(),
            "status": "queued",
            "stepCount": 0,
            "toolCallCount": 0,
            "startedAt": now.to_rfc3339(),
            "updatedAt": now.to_rfc3339(),
            "summary": "",
            "budget": budget,
            "temporaryMode": input.temporary_mode,
        })
        .to_string(),
    )
    .bind(&input.conversation_id)
    .bind(&request_key)
    .execute(&mut *tx)
    .await
    .map_err(crate::error::cmd_err)?;

    // Branch from the edited turn inside the same transaction. The delete and
    // the history read below run on one connection, so the uncommitted delete is
    // visible here; a rollback restores the tail intact.
    if let Some(tail) = truncate {
        truncate_messages_from(
            &mut tx,
            &input.conversation_id,
            &tail.created_at,
            tail.rowid,
        )
        .await?;
    }

    // Before the user turn is inserted. `load_history` selects every message,
    // so loading it afterwards included the row just written and the engine
    // appended the same text again — every turn was sent to the model twice.
    let history: Vec<HistoryMessage> = load_history(&mut *tx, &input.conversation_id).await?;

    // Persist the user turn.
    let user_data = serde_json::json!({
        "role": "user",
        "content": input.content,
        "modelId": model_id,
        "runId": run_id.to_string(),
        "toolCalls": [],
        "attachments": input.attachments,
        "kind": "user",
    });
    sqlx::query("INSERT INTO messages (id, owner_id, data, conversation_id) VALUES (?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(user_data.to_string())
        .bind(&input.conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;

    // Touch the conversation so it sorts to the top.
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE conversations SET data = json_set(data, '$.updatedAt', ?) WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(&input.conversation_id)
        .execute(&mut *tx)
        .await,
        "touch conversation",
    );

    // Truncate + Run + user turn are now committed together; only past this
    // point may the engine start and read the transcript.
    tx.commit().await.map_err(crate::error::cmd_err)?;

    // Snapshot the Git worktree before any tool runs. Best-effort: a non-Git
    // workspace simply keeps the engine's edit/files checkpoints.
    crate::workspace_baseline::capture_and_store(pool, baseline_root.as_deref(), run_id).await;

    // A temporary turn with no earlier messages created its conversation; once
    // the Run ends there is nothing to return to, so the thread goes too.
    let temporary_mode = input.temporary_mode;
    let purge_conversation = temporary_mode && history.is_empty();

    // A local model's advertised context is a maximum, not a budget. Apply the
    // user's ceiling and hand Ollama the resulting `num_ctx`, and carry the
    // keep-alive preference for the native Ollama transport.
    let locality = crate::commands::providers::provider_locality(pool, &provider_id).await;
    let local_settings = crate::commands::settings::read_settings(pool).await.ok();
    let max_window = crate::commands::projects::context_window_for(pool, &model_id).await;
    let context_window = crate::commands::projects::effective_context_window(
        max_window,
        locality.is_managed,
        local_settings
            .as_ref()
            .and_then(|settings| settings.local_context_window),
    );
    let keep_alive = if locality.is_managed && locality.is_ollama {
        local_settings
            .as_ref()
            .and_then(|settings| settings.local_keep_alive.clone())
    } else {
        None
    };

    let request = RunRequest {
        run_id,
        conversation_id: Uuid::parse_str(&input.conversation_id).map_err(crate::error::cmd_err)?,
        workspace_id,
        root_path: project.root_path,
        additional_paths: project.additional_paths,
        mode,
        model_id: model_id.clone(),
        provider_id: provider_id.clone(),
        system: project.system,
        verify_command: project.verify_command,
        format_command: project.format_command,
        context_window,
        temperature: input.temperature,
        reasoning,
        max_tokens: input.max_tokens,
        keep_alive,
        temporary: temporary_mode,
        tools,
        history,
        user_message: input.content,
        resume_from: None,
        auto_fallbacks,
    };

    let engine = state.engine.clone();
    let pool = pool.clone();
    let conversation_id = input.conversation_id.clone();
    // Moved into the background task: stamping Auto provenance and learning
    // from the outcome both need the final status.
    let auto_stamp = auto_selection.clone();
    let auto_model_id = model_id.clone();
    let auto_provider_id = provider_id.clone();
    let auto_was_requested = is_auto;
    tokio::spawn(async move {
        // Run the engine in a child task so a panic there is observable as a
        // JoinError instead of silently aborting this task and leaving the
        // run row `running` forever.
        let engine_for_task = engine.clone();
        let child = tokio::spawn(async move { engine_for_task.execute(request).await });
        let result = child.await;
        // A fallback changes which model actually served the Run. Update the
        // provenance before stamping messages so feedback is attributed to the
        // model that answered, not the one that failed.
        let mut effective_selection = auto_stamp.clone();
        let mut final_model_id = auto_model_id.clone();
        let mut final_provider_id = auto_provider_id.clone();
        let fallback_attempts = match &result {
            Ok(Ok(outcome)) => outcome.auto_fallbacks.clone(),
            _ => Vec::new(),
        };
        if let Some(last) = fallback_attempts.last() {
            final_model_id = last.to_model_id.clone();
            final_provider_id = last.to_provider_id.clone();
            if let Some(selection) = &mut effective_selection {
                selection.selected_id = final_model_id.clone();
                selection.reason = format!(
                    "{}。フォールバック: {} → {}（{}）",
                    selection.reason,
                    last.from_model_id,
                    last.to_model_id,
                    last.failure_kind.as_str()
                );
            }
        }
        // Stamp Auto provenance onto this run's assistant turns so feedback
        // can attribute the rating to the resolved model + route profile.
        // The engine owns transcript interleaving, so this happens after it
        // finishes rather than during.
        if let Some(selection) = &effective_selection {
            let selection_json =
                serde_json::to_string(selection).unwrap_or_else(|_| "{}".to_string());
            mixd_storage::log_write(
                sqlx::query(
                    "UPDATE messages SET data = json_set(data, '$.autoSelection', json(?)) \
                     WHERE conversation_id = ? AND json_extract(data, '$.runId') = ? \
                     AND json_extract(data, '$.role') = 'assistant'",
                )
                .bind(&selection_json)
                .bind(&conversation_id)
                .bind(run_id.to_string())
                .execute(&pool)
                .await,
                "stamp auto selection",
            );
        }
        // The Run row was written before the engine ran, with the primary
        // model. Record the model that actually served it and the fallback
        // trail so history and diagnostics agree with the transcript.
        if !fallback_attempts.is_empty() {
            let attempts_json =
                serde_json::to_string(&fallback_attempts).unwrap_or_else(|_| "[]".to_string());
            mixd_storage::log_write(
                sqlx::query(
                    "UPDATE runs SET data = json_set(data, '$.modelId', ?, \
                     '$.providerId', ?, '$.autoFallbacks', json(?)) WHERE id = ?",
                )
                .bind(&final_model_id)
                .bind(&final_provider_id)
                .bind(&attempts_json)
                .bind(run_id.to_string())
                .execute(&pool)
                .await,
                "record auto fallback",
            );
            if let Some(selection_json) = effective_selection
                .as_ref()
                .and_then(|selection| serde_json::to_string(selection).ok())
            {
                mixd_storage::log_write(
                    sqlx::query(
                        "UPDATE runs SET data = json_set(data, '$.autoSelection', json(?)) \
                         WHERE id = ?",
                    )
                    .bind(&selection_json)
                    .bind(run_id.to_string())
                    .execute(&pool)
                    .await,
                    "refresh auto selection after fallback",
                );
            }
        }
        // Learn availability: one lightweight event per Auto turn keeps future
        // routing away from models/providers that just failed.
        if auto_was_requested {
            let profile = auto_stamp
                .as_ref()
                .map(|s| s.profile.as_str())
                .unwrap_or("");
            let required = auto_stamp.as_ref().map(|s| s.required_tokens);
            match &result {
                Ok(Ok(outcome)) => {
                    for perf in &outcome.turn_performances {
                        crate::commands::auto::record_turn_performance(
                            &pool,
                            profile,
                            mode.as_str(),
                            perf,
                        )
                        .await;
                    }
                    // A failed Run with no recorded turn (engine-level failure
                    // before the first provider call) still needs one event.
                    let recorded_failure = outcome
                        .turn_performances
                        .iter()
                        .any(|perf| perf.failure_kind.is_some());
                    if outcome.status == RunStatus::Failed && !recorded_failure {
                        crate::commands::auto::record_auto_outcome(
                            &pool,
                            &final_model_id,
                            &final_provider_id,
                            mode.as_str(),
                            "failure",
                            Some("other"),
                            Some(profile),
                            required,
                        )
                        .await;
                    }
                }
                Ok(Err(err)) => {
                    let classification =
                        mixd_providers::reliability::ProviderErrorKind::from_message(
                            &err.to_string(),
                        )
                        .as_str();
                    crate::commands::auto::record_auto_outcome(
                        &pool,
                        &final_model_id,
                        &final_provider_id,
                        mode.as_str(),
                        "failure",
                        Some(classification),
                        Some(profile),
                        required,
                    )
                    .await;
                }
                Err(_) => {
                    crate::commands::auto::record_auto_outcome(
                        &pool,
                        &final_model_id,
                        &final_provider_id,
                        mode.as_str(),
                        "failure",
                        Some("other"),
                        Some(profile),
                        required,
                    )
                    .await;
                }
            }
        }
        match result {
            Ok(Ok(outcome)) => {
                mixd_storage::log_write(
                    sqlx::query("UPDATE runs SET status = ? WHERE id = ?")
                        .bind(outcome.status.as_str())
                        .bind(run_id.to_string())
                        .execute(&pool)
                        .await,
                    "record final run status",
                );
                // The assistant turn is already in `messages`: the engine writes
                // the whole transcript, including tool-call turns and results,
                // because it is the only place that knows the correct
                // interleaving. Writing the final text again here would duplicate
                // it and drop the tool history that a later turn needs.
            }
            Ok(Err(err)) => {
                tracing::error!(?err, %run_id, "run failed");
                record_run_failed(&pool, &engine, run_id, &err.to_string()).await;
            }
            Err(join_err) => {
                tracing::error!(?join_err, %run_id, "run task panicked or was cancelled");
                record_run_failed(
                    &pool,
                    &engine,
                    run_id,
                    &format!("run task failed: {join_err}"),
                )
                .await;
            }
        }

        if temporary_mode {
            if let Err(err) = purge_temporary_run(
                &pool,
                &conversation_id,
                &run_id.to_string(),
                purge_conversation,
            )
            .await
            {
                tracing::warn!(?err, "failed to purge a temporary run");
            }
        }
    });

    Ok(SendMessageOutput {
        run_id: run_id.to_string(),
    })
}
