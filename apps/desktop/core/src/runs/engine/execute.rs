use super::helpers::*;
use super::*;

impl RunEngine {
    pub async fn execute(self: &Arc<Self>, request: RunRequest) -> anyhow::Result<RunOutcome> {
        let run_id = request.run_id;
        let provider = self
            .providers
            .get(&request.provider_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("provider not configured: {}", request.provider_id))?;

        let cancellation = RunCancellation::new();
        self.active
            .lock()
            .await
            .insert(run_id, cancellation.clone());

        let mut budget = Budget::for_mode(request.mode);
        // The stored budget is an override *on top of* the mode's, not a
        // replacement for it: the shell writes `null` for any limit the user did
        // not edit, and treating that as the global default would shrink an
        // agent run from 300 steps to 12 the moment anyone granted an extension.
        if let Some(stored) = budget_snapshot(&run_id, self).await {
            let base = budget.clone();
            budget = Budget::from_json_with_base(&stored, base);
        }

        let mut messages: Vec<Message> = request
            .resume_from
            .as_ref()
            .map(|s| s.messages.iter().map(history_to_message).collect())
            .unwrap_or_default();
        if request.resume_from.is_none() {
            for h in &request.history {
                messages.push(history_to_message(h));
            }
            messages.push(Message::text("user", request.user_message.clone()));
            self.emit(
                run_id,
                "message",
                serde_json::json!({ "role": "user", "content": request.user_message }),
            )
            .await;

            // Inject relevant Memory / Knowledge once, at the head of the Run.
            // A temporary turn skips retrieval entirely: the point of the mode
            // is that this exchange leaves no trace, and a memory written from
            // it would do exactly the opposite.
            if !request.temporary {
                match crate::retrieval::retrieve_all(self.storage.as_ref(), &request.user_message)
                    .await
                {
                    Ok(block) => {
                        if let Some(msg) = block.to_message() {
                            self.emit(
                                run_id,
                                "context_summary",
                                serde_json::json!({
                                    "memories": block.memories.len(),
                                    "knowledge": block.knowledge.len(),
                                }),
                            )
                            .await;
                            messages.insert(0, msg);
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%run_id, %err, "memory/knowledge retrieval failed; continuing without injected context");
                    }
                }
            }
        }

        self.set_run_state(run_id, RunStatus::Running, serde_json::json!({}))
            .await;
        self.emit(
            run_id,
            "status",
            serde_json::json!({ "status": "running", "mode": request.mode.as_str() }),
        )
        .await;
        self.emit(
            run_id,
            "model_selected",
            serde_json::json!({ "model_id": request.model_id, "provider_id": request.provider_id }),
        )
        .await;

        let mut steps: u32 = request
            .resume_from
            .as_ref()
            .map(|s| s.step_index)
            .unwrap_or(0);
        let mut total_tool_calls: u32 = request
            .resume_from
            .as_ref()
            .map(|s| s.tool_calls)
            .unwrap_or(0);
        let mut all_tool_calls: Vec<RunToolCall> = Vec::new();
        let mut final_text = String::new();
        let mut final_reasoning = String::new();
        let mut last_usage: Option<Usage> = None;
        let mut last_finish: Option<String> = None;
        let mut stagnation = StagnationDetector::new();
        let mut verification = VerificationLedger::new(request.verify_command.clone());
        // Signals for the current turn, collected as tool calls execute. Kept
        // out here because the classifier needs the tool result, which only
        // exists after dispatch.
        let mut signals: Vec<crate::runs::stagnation::TurnSignal> = Vec::new();
        // How many extra turns the agent has been given to verify an
        // unconfirmed destructive call. Bounded so a model that never reads
        // anything back cannot loop forever.
        let mut unverified_repair_turns: u32 = 0;
        // Whether the engine has already run the project's check automatically.
        // Without this a failing check would re-arm the gate every turn and
        // loop until the step budget is exhausted.
        let mut auto_verified = false;
        // Plan mode is lifted for the rest of the Run once the user approves a
        // `user_plan` proposal. Kept here rather than on the request: it is a
        // property of this execution, not of the conversation.
        let mut plan_approved = false;
        // Last checklist emitted, so the UI only gets an event when it changes.
        let mut last_todos: Vec<mixd_tools::todo::TodoItem> = Vec::new();
        let mut summary = crate::context::TieredSummary::new();
        let task_state = crate::context::TaskState {
            goal: request
                .user_message
                .chars()
                .take(GOAL_PREVIEW_CHARS)
                .collect(),
            ..Default::default()
        };
        // The skill catalogue belongs in the system prompt, not in the history:
        // it is the same on every turn and the adapters cache it there.
        //
        // The built-in procedure is composed in here rather than by each caller
        // so every entry point — chat, agent, resumed run, scheduled run,
        // sub-agent — gets it from one place, and a caller that passes no system
        // text no longer sends an empty prompt.
        let mut system = crate::prompt::compose_system_for_run(
            request.system.as_deref(),
            request.mode,
            !request.tools.is_empty(),
        )
        .unwrap_or_default();
        // Snapshot the registry before awaiting its catalogue: holding the read
        // guard across `skills_prompt(...).await` blocked an MCP reconnect's
        // write lock for the whole lookup.
        let registry = self.tools.read().await.clone();
        let skills_prompt = registry.skills_prompt(request.root_path.as_deref()).await;
        if !skills_prompt.trim().is_empty() {
            if !system.trim().is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(&skills_prompt);
        }
        if !request.tools.is_empty() {
            let allowed: Vec<&str> = request.tools.iter().map(|t| t.name.as_str()).collect();
            system.push_str(&format!(
                "\n<tool_policy>\nFor this run you may call only: {}. Anything else will be \
                 rejected.\n</tool_policy>",
                allowed.join(", ")
            ));
        }

        loop {
            if cancellation.is_cancelled() {
                return self
                    .finish_cancelled(
                        run_id,
                        request.conversation_id,
                        &request.model_id,
                        final_text,
                        final_reasoning,
                        all_tool_calls,
                        last_usage,
                        last_finish,
                        steps,
                    )
                    .await;
            }

            // Fold any message the user typed while the Run was working into
            // the transcript. It becomes an ordinary user turn, so the model
            // sees it exactly where it would have seen a message sent between
            // Runs, and the persisted transcript matches what the UI showed.
            for text in self.take_steering(run_id).await {
                let message = Message::text("user", format!("[user interjection] {text}"));
                self.append_message(
                    request.conversation_id,
                    run_id,
                    &request.model_id,
                    &message_to_history(&message),
                )
                .await;
                messages.push(message);
                self.emit(
                    run_id,
                    "steering_applied",
                    serde_json::json!({ "content": text }),
                )
                .await;
            }

            // Budget gate before each Provider turn.
            match budget.check(steps, total_tool_calls).await {
                BudgetVerdict::Ok => {}
                BudgetVerdict::Exhausted(reason) => {
                    let Some(grant) = self.await_budget_extension(run_id, reason, &budget).await
                    else {
                        // Cancelled while waiting: reporting `completed` here
                        // overwrote the user's cancellation.
                        return self
                            .finish_cancelled(
                                run_id,
                                request.conversation_id,
                                &request.model_id,
                                final_text,
                                final_reasoning,
                                all_tool_calls,
                                last_usage,
                                last_finish,
                                steps,
                            )
                            .await;
                    };
                    // A granted extension can still be refused by the grant
                    // allowance, so the two outcomes are distinguished rather
                    // than both reported as "declined".
                    let extended = grant && budget.extend();
                    let stop_reason = if !grant {
                        "budget_declined"
                    } else {
                        "budget_grants_exhausted"
                    };
                    if extended {
                        self.set_run_state(run_id, RunStatus::Running, serde_json::json!({}))
                            .await;
                    } else {
                        return self
                            .finish_run(
                                run_id,
                                RunStatus::Completed,
                                serde_json::json!({}),
                                serde_json::json!({ "status": "completed", "reason": stop_reason }),
                                RunOutcome {
                                    status: RunStatus::Completed,
                                    text: final_text,
                                    reasoning: final_reasoning,
                                    tool_calls: all_tool_calls,
                                    usage: last_usage,
                                    finish_reason: last_finish,
                                    steps,
                                },
                            )
                            .await;
                    }
                }
            }

            steps += 1;

            // Hierarchical context compression. The head block — the task goal
            // plus the agent's own checklist — is prepended so both survive the
            // folding of older turns. Without the checklist an agent loses track
            // of a five-step refactor as soon as the context compresses.
            //
            // The head and the checklist are read under one shared guard, and
            // only once per step: rendering the block twice meant formatting
            // the whole checklist two times and taking the registry lock three
            // times on every pass through the loop. Deriving both from one
            // snapshot is what keeps the plan the model is acting on identical
            // to the plan the UI shows.
            let (head, checklist) = {
                // Clone the checklist handle while the registry guard is held,
                // then drop it: `render_head` and `snapshot` await the store's
                // own lock, and holding the registry read guard across them
                // stalled an MCP reconnect's write lock for the whole render.
                let (todos, tiered) = {
                    let tools = self.tools.read().await;
                    (tools.todos.clone(), self.policy.context_tiered_summary)
                };
                let head = if tiered {
                    render_head(&task_state, &todos, run_id).await
                } else {
                    String::new()
                };
                (head, todos.snapshot(run_id).await)
            };
            // Surface the checklist to the UI, not just to the model: a plan the
            // user cannot see is a plan they cannot correct.
            if checklist != last_todos {
                let payload = serde_json::json!({
                    "todos": checklist
                        .iter()
                        .map(|item| serde_json::json!({
                            "id": item.id,
                            "content": item.content,
                            "status": item.status,
                        }))
                        .collect::<Vec<_>>(),
                });
                last_todos = checklist;
                self.emit(run_id, "todo_list", payload).await;
            }

            // One history list for the rest of the step: the compression pass
            // and the request below both work on it, and the Provider takes it
            // over afterwards. This used to be two separate clones of the whole
            // conversation per step, plus a third inside `compress`.
            let mut current = if head.trim().is_empty() {
                messages.clone()
            } else {
                let mut v = Vec::with_capacity(messages.len() + 1);
                v.push(Message::text("system", head));
                v.extend(messages.iter().cloned());
                v
            };

            if self.policy.context_tiered_summary {
                let source_messages = current.len();
                // The summariser opens its own Provider turn, so it needs the
                // bare model name for the same reason `run_turn` does.
                let model = request.wire_model().to_string();
                let provider_handle = provider.clone();
                // The summariser is handed exactly the slice being dropped, so
                // the summary never duplicates the tail kept verbatim. The
                // history and the summary are moved in and handed straight
                // back, so an ordinary (uncompressed) step allocates nothing.
                let outcome = crate::context::compress_with_budget(
                    std::mem::take(&mut current),
                    std::mem::take(&mut summary),
                    request.context_window,
                    |older| async move {
                        summarize_turns(provider_handle.as_ref(), &model, &older).await
                    },
                )
                .await;
                let evicted = outcome.evicted_tool_results;
                current = outcome.messages;
                summary = outcome.summary;
                if !outcome.skipped || evicted > 0 {
                    messages = strip_head(&current);
                    self.emit(
                        run_id,
                        "context_summary",
                        serde_json::json!({
                            "compressed": !outcome.skipped,
                            "activeChars": summary.char_count(),
                            "archiveRows": summary.archive.len(),
                            "messages": messages.len(),
                            "sourceMessages": source_messages,
                            "evictedToolResults": evicted,
                        }),
                    )
                    .await;
                }
            }

            let turn = self
                .run_turn(run_id, provider.as_ref(), &request, current, &system)
                .await?;

            // A cancellation observed mid-stream comes back as `turn.error`;
            // report it as Cancelled rather than as a failure. `run_turn` keeps
            // the deltas it had already streamed in `turn.text/reasoning`, so
            // those are what get persisted — `final_text` only holds a turn
            // that was already written to the transcript.
            if cancellation.is_cancelled() {
                return self
                    .finish_cancelled(
                        run_id,
                        request.conversation_id,
                        &request.model_id,
                        turn.text,
                        turn.reasoning,
                        all_tool_calls,
                        last_usage,
                        last_finish,
                        steps,
                    )
                    .await;
            }

            if let Some(err) = turn.error {
                self.emit(run_id, "error", serde_json::json!({ "error": err }))
                    .await;
                return self
                    .finish_run(
                        run_id,
                        RunStatus::Failed,
                        serde_json::json!({ "error": err }),
                        serde_json::json!({ "status": "failed", "error": err }),
                        RunOutcome {
                            status: RunStatus::Failed,
                            text: final_text,
                            reasoning: final_reasoning,
                            tool_calls: all_tool_calls,
                            usage: last_usage,
                            finish_reason: last_finish,
                            steps,
                        },
                    )
                    .await;
            }

            // Spend the turn against the token ceiling. `last_usage` is the
            // *latest* turn's usage, which the UI shows per turn; the budget
            // needs the running total.
            if let Some(usage) = &turn.usage {
                budget.record_usage(usage.input_tokens, usage.output_tokens);
            }
            last_usage = turn.usage.clone();
            last_finish = turn.finish_reason.clone();
            if !turn.reasoning.is_empty() {
                final_reasoning = turn.reasoning.clone();
            }

            // Stagnation detection: inject a repair hint as a user-role turn.
            //
            // A turn that produced no tool calls and no error is progress, and
            // the loop ends below; anything else is fed in as-is.
            if turn.tool_calls.is_empty() && turn.error.is_none() {
                signals.clear();
                signals.push(crate::runs::stagnation::TurnSignal::Progress);
            }
            // `take`, not `clone`: the detector sees each signal exactly once.
            // Cloning let every turn's signals be re-fed on every later turn, so
            // the window filled with duplicates after three real failures and
            // kept firing the repair hint forever.
            let turn_signals = std::mem::take(&mut signals);
            if self.policy.stagnation_detection {
                if let Some(report) = stagnation.observe(turn_signals) {
                    self.emit(
                        run_id,
                        "stagnation_detected",
                        serde_json::json!({ "hint": report.hint, "kind": report.kind }),
                    )
                    .await;

                    // Phase 8.2: Turn-level re-retrieval on stagnation
                    if !request.temporary {
                        let query = format!("{} {}", request.user_message, report.hint);
                        if let Ok(block) =
                            crate::retrieval::retrieve_all(self.storage.as_ref(), &query).await
                        {
                            if let Some(msg) = block.to_message() {
                                messages.push(msg);
                            }
                        }
                    }

                    messages.push(Message::text(
                        "user",
                        format!(
                            "[self-repair hint] {} — adjust the approach before retrying.",
                            report.hint
                        ),
                    ));
                }
            }

            if turn.tool_calls.is_empty() {
                // Strict verification, enforced. A destructive call that was
                // never read back leaves the ledger open, and the model is not
                // allowed to conclude: it gets one more turn to verify the
                // effect. Previously the ledger was written and never read, so
                // this branch could not fire.
                // Two independent questions, both gated here: "did you read the
                // effect back?" (`is_blocked`) and "did you run the project's
                // own check?" (`needs_verification`). Reading a file back
                // correctly proves nothing about whether it still builds.
                let unanswered = verification.is_blocked();
                let unchecked = verification.needs_verification();
                if self.policy.strict_verification && (unanswered || unchecked) {
                    if unverified_repair_turns >= MAX_UNVERIFIED_REPAIR_TURNS {
                        self.emit(
                            run_id,
                            "verification_failed",
                            serde_json::json!({
                                "outstanding": verification.outstanding(),
                                "unverified": verification.unverified(),
                                "reason": verification.last_reason(),
                            }),
                        )
                        .await;
                        final_text = turn.text.clone();
                        let final_message = Message::text("assistant", turn.text.clone());
                        // Persist it: `messages` is in-memory only, and the next
                        // turn of the conversation is rebuilt from this table.
                        self.append_message(
                            request.conversation_id,
                            run_id,
                            &request.model_id,
                            &message_to_history(&final_message),
                        )
                        .await;
                        messages.push(final_message);
                        if !turn.text.is_empty() {
                            self.emit(
                                run_id,
                                "message",
                                serde_json::json!({
                                    "role": "assistant",
                                    "content": turn.text,
                                    "model_id": request.model_id,
                                }),
                            )
                            .await;
                        }
                        if !request.temporary {
                            self.spawn_memory_extraction(
                                run_id,
                                &request.user_message,
                                &final_text,
                            );
                        }
                        break;
                    }
                    unverified_repair_turns += 1;
                    let outstanding = verification.outstanding();
                    let unverified = verification.unverified();
                    self.emit(
                        run_id,
                        "verification_required",
                        serde_json::json!({
                            "tool": outstanding.first().cloned().unwrap_or_default(),
                            "outstanding": outstanding,
                            "unverified": unverified,
                            "verifyCommand": verification.verify_command(),
                            "reason": verification.last_reason(),
                        }),
                    )
                    .await;
                    let mut hints: Vec<String> = Vec::new();
                    if unanswered {
                        hints.push(format!(
                            "You have not confirmed the effect of {}. Read the affected path \
                             back and state what is actually there before you conclude. \
                             Outstanding: {}.",
                            outstanding.join(", "),
                            verification
                                .last_reason()
                                .unwrap_or_else(|| "unconfirmed change".to_string())
                        ));
                    }
                    if unchecked {
                        let command = verification.verify_command().unwrap_or_default();
                        let format_note = request
                            .format_command
                            .as_deref()
                            .map(|cmd| format!(" Run `{cmd}` first if you have not."))
                            .unwrap_or_default();
                        hints.push(format!(
                            "You changed {} file(s) but have not run the project check. Run \
                             `{}` and report what it actually said before you conclude.{}\n\
                             Unverified: {}.",
                            unverified.len(),
                            command,
                            format_note,
                            unverified.join(", ")
                        ));
                    }
                    // Phase 8.2: Turn-level re-retrieval on verification checkpoint
                    if !request.temporary {
                        let query = format!("{} verify fix repair", request.user_message);
                        if let Ok(block) =
                            crate::retrieval::retrieve_all(self.storage.as_ref(), &query).await
                        {
                            if let Some(msg) = block.to_message() {
                                messages.push(msg);
                            }
                        }
                    }

                    messages.push(Message::text(
                        "user",
                        format!("[verification] {}", hints.join(" ")),
                    ));
                    continue;
                }
                final_text = turn.text.clone();
                let final_message = Message::text("assistant", turn.text.clone());
                self.append_message(
                    request.conversation_id,
                    run_id,
                    &request.model_id,
                    &message_to_history(&final_message),
                )
                .await;
                messages.push(final_message);
                if !turn.text.is_empty() {
                    self.emit(
                        run_id,
                        "message",
                        serde_json::json!({
                            "role": "assistant",
                            "content": turn.text,
                            "reasoning": turn.reasoning,
                            "model_id": request.model_id,
                        }),
                    )
                    .await;
                }
                if !request.temporary {
                    self.spawn_memory_extraction(run_id, &request.user_message, &final_text);
                }
                break;
            }

            // Assistant turn requested Tools — record it and execute them.
            let assistant_message =
                Message::assistant_with_tools(turn.text.clone(), turn.tool_calls.clone());
            self.append_message(
                request.conversation_id,
                run_id,
                &request.model_id,
                &message_to_history(&assistant_message),
            )
            .await;
            messages.push(assistant_message);
            if !turn.text.is_empty() {
                self.emit(
                    run_id,
                    "message",
                    serde_json::json!({ "role": "assistant", "content": turn.text, "partial": true }),
                )
                .await;
            }

            // The argument blob is decoded once per call. Dispatch, the
            // stagnation detector and the verification ledger all read the very
            // same object, and each used to decode it again from the wire
            // string — six JSON parses per call, on every call of every step.
            let call_args: Vec<serde_json::Value> = turn
                .tool_calls
                .iter()
                .map(|c| {
                    serde_json::from_str(&c.function.arguments).unwrap_or_else(|err| {
                        tracing::warn!(tool = %c.function.name, %err, "tool arguments are not valid JSON; passing Null");
                        serde_json::Value::Null
                    })
                })
                .collect();

            let cancellation = self.cancellation_of(run_id).await;
            // Whether this step changed a file, so the formatter can run once
            // after the batch instead of after every individual edit.
            let mut step_mutated = false;

            // Read-only calls run concurrently. A model that asks for five
            // greps and three reads used to wait for the sum of their
            // latencies; now it waits for the slowest. Mutating calls stay
            // strictly ordered, and every result is reassembled in the order
            // the model emitted the calls, because providers pair a `tool`
            // message with its call by position and id.
            let mut results: Vec<Option<RunToolCall>> = vec![None; turn.tool_calls.len()];
            let mut destructive: Vec<Option<String>> = vec![None; turn.tool_calls.len()];
            let mut pending_reads: Vec<(usize, AssistantToolCall, serde_json::Value)> = Vec::new();

            for (index, (call, args)) in turn.tool_calls.iter().zip(call_args.iter()).enumerate() {
                if cancellation.is_cancelled() {
                    break;
                }
                total_tool_calls += 1;
                if crate::runs::stagnation::is_read_only(&call.function.name, args)
                    && stagnation
                        .note_read(crate::runs::stagnation::read_key(&call.function.name, args))
                {
                    signals.push(crate::runs::stagnation::TurnSignal::RepeatedRead);
                }
                // Register the destructive effect *before* dispatch so a read
                // that follows it in the same turn can clear it. Doing this
                // after the loop meant a `write` then `read` in one assistant
                // turn left the gate open even though the read had confirmed it.
                let effect = destructive_reason(&call.function.name, args);
                destructive[index] = effect.clone();
                if self.policy.strict_verification && effect.is_some() {
                    verification.requires_confirmation(&call.function.name, args);
                    self.emit(
                        run_id,
                        "destructive_tool_called",
                        serde_json::json!({
                            "tool": call.function.name,
                            "arguments": args,
                            "reason": effect.clone().unwrap_or_default(),
                        }),
                    )
                    .await;
                }
                // Plan mode refuses anything that changes the workspace, before
                // the permission gate and therefore before any stored allow
                // rule can turn it writable. A refusal is reported through the
                // same denied-call path as a user denial, so the model gets a
                // reason it can act on instead of a silent no-op.
                if request.mode.is_read_only()
                    && !plan_approved
                    && crate::runs::stagnation::is_mutating(&call.function.name, args)
                {
                    let record = RunToolCall {
                        id: call.id.clone(),
                        name: call.function.name.clone(),
                        arguments: args.clone(),
                        result: None,
                        error: None,
                        approval_id: None,
                    };
                    results[index] = Some(
                        self.deny_tool(
                            run_id,
                            record,
                            "plan mode is read-only; describe the change and present it \
                             with `user_plan` instead of making it",
                        )
                        .await,
                    );
                    continue;
                }
                // Refuse to edit content the model has not seen. Only a file it
                // actually read is checked, so a fresh edit with no prior read
                // keeps working; the dangerous case is the file changing on
                // disk after the read and the anchor still matching stale text.
                if let Some(reason) = self
                    .stale_write_reason(
                        run_id,
                        request.root_path.as_deref(),
                        &call.function.name,
                        args,
                    )
                    .await
                {
                    let record = RunToolCall {
                        id: call.id.clone(),
                        name: call.function.name.clone(),
                        arguments: args.clone(),
                        result: None,
                        error: None,
                        approval_id: None,
                    };
                    results[index] = Some(self.fail_tool(run_id, record, &reason).await);
                    continue;
                }
                // Capture the pre-Run content before the first change, so the
                // whole Run can be undone. Read-only calls have no paths.
                let paths = mutated_paths(&call.function.name, args);
                if !paths.is_empty() {
                    self.snapshot_paths(run_id, request.root_path.as_deref(), &paths)
                        .await;
                }
                if can_run_concurrently(&call.function.name, args) {
                    pending_reads.push((index, call.clone(), args.clone()));
                    continue;
                }
                // A mutating call may depend on nothing, but a read scheduled
                // *after* it in the same turn may depend on its effect. Flush
                // the current batch first so the two cannot race.
                self.flush_concurrent_reads(
                    run_id,
                    &request,
                    &mut pending_reads,
                    &mut results,
                    &cancellation,
                )
                .await;
                // A `terminal` call or a slow fetch must not make the Run
                // uncancellable: race the dispatch against the cancellation
                // signal and stop as soon as the user asks to.
                let result = tokio::select! {
                    biased;
                    _ = cancellation.cancelled_signal() => {
                        tracing::info!(%run_id, tool = %call.function.name, "cancelled mid-tool; stopping");
                        break;
                    }
                    result = self.dispatch_tool(run_id, &request, call, args) => result,
                };
                results[index] = Some(result);
            }
            // Anything still buffered (the turn ended, or was cancelled between
            // calls) runs now.
            self.flush_concurrent_reads(
                run_id,
                &request,
                &mut pending_reads,
                &mut results,
                &cancellation,
            )
            .await;

            // Post-processing is a separate pass in call order: the verification
            // ledger, stagnation signals and the transcript must observe the
            // calls in the order the model made them, not the order concurrent
            // reads happened to finish.
            let mut tool_messages: Vec<Message> = Vec::new();
            let mut plan_handoff = false;
            for (index, (call, args)) in turn.tool_calls.iter().zip(call_args.iter()).enumerate() {
                let Some(result) = results[index].take() else {
                    // The Run was cancelled before this call ran. The assistant
                    // turn already declared it, so it still needs a terminal
                    // result or the persisted transcript is invalid for every
                    // provider (a dangling `tool_calls` turn is rejected).
                    tool_messages.push(Message::tool_result(call, "Error: run cancelled", true));
                    continue;
                };
                all_tool_calls.push(result.clone());
                let is_error = result.error.is_some();
                if !is_error
                    && call.function.name == "files"
                    && args.get("action").and_then(|v| v.as_str()) == Some("read")
                {
                    if let (Some(path), Some(fingerprint)) = (
                        args.get("path").and_then(|v| v.as_str()),
                        result
                            .result
                            .as_ref()
                            .and_then(|payload| payload.get("fingerprint"))
                            .and_then(|value| value.as_u64()),
                    ) {
                        if fingerprint != 0 {
                            let key = normalize_tool_path(
                                request.root_path.as_deref().map(std::path::Path::new),
                                path,
                            );
                            self.read_fingerprints
                                .lock()
                                .await
                                .entry(run_id)
                                .or_default()
                                .insert(key, fingerprint);
                        }
                    }
                }
                // The user approved the proposal: lift Plan mode for the rest
                // of this Run and tell the model that it may now make changes.
                if request.mode == Mode::Plan && call.function.name == "user_plan" && !is_error {
                    plan_approved = true;
                    plan_handoff = true;
                }
                // Only a read that actually succeeded closes the gate, and only
                // for the target it confirms.
                if !is_error && destructive[index].is_none() {
                    verification.confirm(&call.function.name, args);
                }
                if !is_error {
                    // A deliberate file change re-arms the check gate, and a
                    // successful `terminal` run of the configured check is the
                    // only thing that clears it.
                    let mutated = mutated_paths(&call.function.name, args);
                    if !mutated.is_empty() {
                        step_mutated = true;
                    }
                    for path in mutated {
                        verification.note_mutation(&path);
                    }
                    if call.function.name == "terminal" {
                        let exit_ok = result
                            .result
                            .as_ref()
                            .and_then(exit_code_of)
                            .is_some_and(|code| code == 0);
                        verification.note_command(
                            args.get("command")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default(),
                            exit_ok,
                        );
                    }
                }
                let content = match &result.error {
                    Some(err) => format!("Error: {}", err),
                    None => result
                        .result
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "ok".to_string()),
                };
                // The failure flag is what the adapters turn into their own error
                // channel and what the stagnation detector reads, so it has to be
                // set here rather than inferred from the message text later.
                signals.push(classify_tool_call(&call.function.name, args, is_error));
                tool_messages.push(Message::tool_result(call, content, is_error));
            }
            for message in &tool_messages {
                self.append_message(
                    request.conversation_id,
                    run_id,
                    &request.model_id,
                    &message_to_history(message),
                )
                .await;
            }
            messages.extend(tool_messages);
            if plan_handoff {
                messages.push(Message::text(
                    "user",
                    "[plan approved] The user approved this plan. Plan mode is lifted for \
                     the rest of this Run: make the changes now, run the project's check, \
                     and report what you actually verified.",
                ));
            }

            // Auto-format: run the project's formatter/lint command after a
            // step that changed files. The command was previously only
            // mentioned in the verification hint, so a configured formatter
            // never actually ran. A failure is surfaced to the model as a user
            // turn; success is only reported as an event.
            if step_mutated {
                if let Some(command) = request
                    .format_command
                    .as_deref()
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                {
                    let command = command.to_string();
                    let ctx = ToolContext {
                        run_id,
                        workspace_id: request.workspace_id,
                        root_path: request.root_path.clone(),
                        additional_paths: request.additional_paths.clone(),
                        approval_token: None,
                    };
                    let result = registry
                        .dispatch(ctx, "terminal", serde_json::json!({ "command": command }))
                        .await;
                    let (ok, output) = match result {
                        Ok(r) => {
                            let exit_ok = r
                                .content
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .is_some_and(|c| c == 0);
                            (r.ok && exit_ok, r.content.to_string())
                        }
                        Err(e) => (false, format!("Error: {e}")),
                    };
                    self.emit(
                        run_id,
                        "format_applied",
                        serde_json::json!({
                            "command": command,
                            "ok": ok,
                            "output": truncate_event_text(&output),
                        }),
                    )
                    .await;
                    if !ok {
                        messages.push(Message::text(
                            "user",
                            format!(
                                "[auto-format] `{command}` failed. Fix the cause before you conclude:\n{output}"
                            ),
                        ));
                    }
                }
            }

            // Auto-verify: when the project's check is configured and the
            // agent has changed files without running it, run the check
            // automatically. The model-driven hint is still emitted (so the
            // model can run it itself if it prefers), but the engine no longer
            // depends on the model noticing the hint. A successful run clears
            // the gate via `note_command`; a failing run leaves it open so the
            // model sees the output and can react.
            //
            // The guard on `auto_verified` prevents an infinite loop: if the
            // check itself fails, the gate stays open and the repair-turn
            // counter (bounded by `MAX_UNVERIFIED_REPAIR_TURNS`) eventually
            // stops the run.
            if self.policy.strict_verification
                && verification.needs_verification()
                && !auto_verified
            {
                if let Some(command) = verification.verify_command() {
                    let command = command.to_string();
                    auto_verified = true;
                    let ctx = ToolContext {
                        run_id,
                        workspace_id: request.workspace_id,
                        root_path: request.root_path.clone(),
                        additional_paths: request.additional_paths.clone(),
                        approval_token: None,
                    };
                    let result = registry
                        .dispatch(ctx, "terminal", serde_json::json!({ "command": command }))
                        .await;
                    let (ok, output) = match result {
                        Ok(r) => {
                            let exit_ok = r
                                .content
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .is_some_and(|c| c == 0);
                            (r.ok && exit_ok, r.content.to_string())
                        }
                        Err(e) => (false, format!("Error: {e}")),
                    };
                    verification.note_command(&command, ok);
                    let summary = if ok {
                        format!("[auto-verify] `{command}` passed.")
                    } else {
                        format!("[auto-verify] `{command}` failed:\n{output}")
                    };
                    messages.push(Message::text("user", summary));
                }
            }

            // A run that has actually verified its effects earns its repair
            // allowance back, so strict verification cannot permanently cap a
            // long multi-file run after two unconfirmed turns.
            if !verification.is_blocked() && !verification.needs_verification() {
                unverified_repair_turns = 0;
            }

            // Intermediate checkpointing. The interval is a public policy field
            // a caller can set, so a zero must not turn the modulo into a panic.
            if self.policy.checkpointing
                && self.policy.checkpoint_every_steps > 0
                && steps % self.policy.checkpoint_every_steps == 0
            {
                let snapshot = CheckpointSnapshot {
                    step_index: steps,
                    tool_calls: total_tool_calls,
                    messages: messages.iter().map(message_to_history).collect(),
                };
                let id = self
                    .checkpoints
                    .save(
                        run_id,
                        steps,
                        total_tool_calls,
                        &snapshot,
                        self.storage.as_ref(),
                    )
                    .await;
                self.emit(
                    run_id,
                    "checkpoint_saved",
                    serde_json::json!({ "checkpoint_id": id, "step": steps }),
                )
                .await;
            }
        }

        self.finish_run(
            run_id,
            RunStatus::Completed,
            serde_json::json!({
                "stepCount": steps,
                "toolCallCount": total_tool_calls,
            }),
            serde_json::json!({
                "status": "completed",
                "steps": steps,
                "tool_calls": total_tool_calls,
                "usage": last_usage,
                "tokensUsed": budget.tokens_used,
                "tokenLimit": budget.token_limit,
                "estimatedCostUsd": last_usage
                    .as_ref()
                    .and_then(|usage| crate::pricing::estimate_cost_usd(request.wire_model(), usage)),
            }),
            RunOutcome {
                status: RunStatus::Completed,
                text: final_text,
                reasoning: final_reasoning,
                tool_calls: all_tool_calls,
                usage: last_usage,
                finish_reason: last_finish,
                steps,
            },
        )
        .await
    }
}

/// The raw stored budget object, so the caller can decide what the mode's
/// budget is before applying the overrides in it.
///
/// A query failure or corrupt blob reads as "no override" with a warn, never
/// as a silent default: without the log a locked database looked identical to
/// a run with no stored budget.
async fn budget_snapshot(run_id: &RunId, engine: &RunEngine) -> Option<serde_json::Value> {
    let row: Option<(String,)> =
        sqlx::query_as::<_, (String,)>("SELECT data FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_optional(engine.storage.pool())
            .await
            .map_err(|err| {
                tracing::warn!(%run_id, %err, "budget_snapshot query failed; using mode default");
                err
            })
            .unwrap_or(None);
    let data: serde_json::Value = row
        .and_then(|(raw,)| {
            serde_json::from_str(&raw)
                .map_err(|err| {
                    tracing::warn!(%run_id, %err, "budget_snapshot saw a corrupt runs.data blob; using mode default");
                })
                .ok()
        })
        .unwrap_or_default();
    data.get("budget").cloned()
}
