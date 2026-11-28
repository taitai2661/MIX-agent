use super::helpers::*;
use super::*;

impl RunEngine {
    pub(super) async fn flush_concurrent_reads(
        &self,
        run_id: RunId,
        request: &RunRequest,
        pending: &mut Vec<(usize, AssistantToolCall, serde_json::Value)>,
        results: &mut [Option<RunToolCall>],
        cancellation: &RunCancellation,
    ) {
        use futures_util::StreamExt;

        if pending.is_empty() {
            return;
        }
        let batch = std::mem::take(pending);
        // `.max(1)` keeps a corrupt/zero policy value from panicking `chunks`.
        for chunk in batch.chunks(self.policy.max_concurrent_reads.max(1)) {
            if cancellation.is_cancelled() {
                break;
            }
            let mut in_flight = futures_util::stream::FuturesUnordered::new();
            for (index, call, args) in chunk {
                in_flight.push(async move {
                    let result = self.dispatch_tool(run_id, request, call, args).await;
                    (*index, result)
                });
            }
            while let Some((index, result)) = in_flight.next().await {
                if cancellation.is_cancelled() {
                    break;
                }
                results[index] = Some(result);
            }
        }
    }

    /// Dispatch a single Tool call, honouring the permission policy.
    #[tracing::instrument(skip_all, fields(run_id = %run_id))]
    pub(super) async fn dispatch_tool(
        &self,
        run_id: RunId,
        request: &RunRequest,
        call: &AssistantToolCall,
        args: &serde_json::Value,
    ) -> RunToolCall {
        // The wire string is decoded once by the caller — the stagnation
        // detector and the verification ledger read the very same object — so
        // this takes the decoded form and copies it into the record it has to
        // persist.
        let arguments = args.clone();
        let mut display_arguments = arguments.clone();
        mixd_tools::redact::redact_arguments(&mut display_arguments);
        let tool_name = call.function.name.clone();
        let tool_call_id = call.id.clone();

        let mut record = RunToolCall {
            id: tool_call_id.clone(),
            name: tool_name.clone(),
            arguments: display_arguments.clone(),
            result: None,
            error: None,
            approval_id: None,
        };

        // A malformed argument blob reaches this function as `Null` (the caller
        // logs the parse error). Dispatching it anyway produced a misleading
        // "path required" from the Tool; report the actual problem instead.
        if arguments.is_null() {
            let raw = call.function.arguments.trim();
            if !raw.is_empty() && raw != "null" {
                return self
                    .fail_tool(
                        run_id,
                        record,
                        &format!("arguments are not valid JSON: {}", truncate_event_text(raw)),
                    )
                    .await;
            }
        }

        // One read of the registry for the whole call, taken as a snapshot
        // rather than a guard. The gate used to take the lock twice here and a
        // third time across the tool execution itself, so a thirty-second
        // `terminal` call held the read lock that an MCP reconnect's write lock
        // is queued behind for the entire command.
        let registry = self.tools.read().await.clone();

        // Permission gate: a stored rule may allow, deny, or prompt.
        //
        // The fallback is per-*call*, not per tool: `files` is one tool but its
        // read and write actions warrant different answers, and an unknown tool
        // is always treated as sensitive rather than optimistically allowed.
        let pattern_target = registry.pattern_target_for(&tool_name, &arguments);
        let default_decision = match registry.get(&tool_name) {
            Some(tool) => tool.decision_for(&arguments),
            None => mixd_tools::permission::PermissionDecision::Ask,
        };
        let permission_ctx = mixd_tools::execute::PermissionContext {
            workspace_id: Some(request.workspace_id),
            pattern_target: pattern_target.as_deref(),
        };
        // A stored rule still wins, so a user can lock a read-only tool down or
        // silence a prompt for a mutating one. Scope and pattern now participate
        // in the lookup: a workspace-scoped deny beats a global allow, and a
        // pattern-scoped rule only matches when the tool exposes a matching
        // target.
        let mut decision = self
            .executor
            .resolve_permission(&tool_name, &permission_ctx, default_decision)
            .await;
        if decision == mixd_tools::permission::PermissionDecision::Deny {
            return self
                .deny_tool(run_id, record, "permission denied by rule")
                .await;
        }

        // Grade the command, not the Tool. A dangerous shell command must
        // never be silently covered by "Always Allow terminal": the rule was
        // written for `cargo test`, not for `rm -rf ~`. Upgrading an Allow to
        // Ask here is the enforcement point; the approval command separately
        // refuses to *create* a durable rule for a dangerous command.
        let risk = mixd_tools::risk::assess_tool_call(&tool_name, &arguments);
        if risk.is_dangerous() && decision == mixd_tools::permission::PermissionDecision::Allow {
            tracing::warn!(
                tool = %tool_name,
                reasons = ?risk.reasons.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(),
                "dangerous command upgraded from allow to explicit approval"
            );
            decision = mixd_tools::permission::PermissionDecision::Ask;
        }

        let mut approval_token: Option<String> = None;
        if decision == mixd_tools::permission::PermissionDecision::Ask {
            // `approvals.tool_call_id` references `tool_calls(id)`, so the call
            // row has to exist before the prompt can be persisted.
            let _ = self
                .executor
                .record_tool_call(
                    run_id,
                    &tool_call_id,
                    &tool_name,
                    display_arguments.clone(),
                    "awaiting_approval",
                    None,
                    None,
                )
                .await;
            let prompt = self
                .executor
                .request_approval_graded(
                    run_id,
                    &tool_call_id,
                    &tool_name,
                    display_arguments.clone(),
                    Some(request.workspace_id),
                    pattern_target.clone(),
                    risk.clone(),
                )
                .await;
            let Ok((mut rx, mut deadline)) = prompt else {
                let reason = format!("could not open approval prompt: {:?}", prompt.err());
                return self.deny_tool(run_id, record, &reason).await;
            };
            let approval = self
                .executor
                .pending
                .lock()
                .await
                .iter()
                .find(|p| p.tool_call_id == tool_call_id)
                .map(|p| p.approval_id.clone());
            record.approval_id = approval.clone();
            approval_token = approval;

            self.set_run_state(
                run_id,
                RunStatus::WaitingApproval,
                serde_json::json!({ "approvalId": record.approval_id }),
            )
            .await;
            self.emit(
                run_id,
                "tool_approval_requested",
                serde_json::json!({
                    "tool_call_id": tool_call_id,
                    "approval_id": record.approval_id,
                    "name": tool_name,
                    "arguments": display_arguments,
                    // The prompt auto-denies at this instant. Without the
                    // deadline the card looked answerable forever and the run
                    // silently continued as "denied" while the user was away.
                    "expires_at": deadline.to_rfc3339(),
                    // Command-level grading so the card can show *why* this is
                    // different from the usual `terminal` prompt.
                    "risk": risk,
                }),
            )
            .await;

            // Wait for the user's answer, the prompt TTL, or cancellation.
            // The deadline is re-read when it fires: an extension granted
            // while this task slept must move the wait, not deny the call.
            let cancellation = self.cancellation_of(run_id).await;
            let outcome = loop {
                let ttl = (deadline - chrono::Utc::now())
                    .to_std()
                    .unwrap_or(std::time::Duration::from_secs(0));
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled_signal() => break ApprovalOutcome::Deny,
                    Ok(outcome) = &mut rx => break outcome,
                    _ = tokio::time::sleep(ttl) => {
                        let extended = match approval_token.as_deref() {
                            Some(id) => self.executor.approval_expiry(id).await,
                            None => None,
                        };
                        match extended {
                            Some(next) if next > chrono::Utc::now() => deadline = next,
                            _ => break ApprovalOutcome::Deny,
                        }
                    }
                }
            };
            // A cancellation may have written a terminal status while the
            // card was open. Only resume when the row is still in the state
            // this prompt put it in, otherwise the unconditional write would
            // turn a Deny/cancel back into `running` (the `ask_user` path
            // below already uses the guarded variant).
            self.set_run_state_if(
                run_id,
                RunStatus::WaitingApproval,
                RunStatus::Running,
                serde_json::json!({}),
            )
            .await;

            // Every outcome has to retire the card: the reducer only clears
            // `timeline.approval` on this event, so a denial that emitted
            // nothing left a live "approve / deny" card on screen for the rest
            // of the run.
            let granted = matches!(outcome, ApprovalOutcome::Approve { .. });
            self.emit(
                run_id,
                "tool_approval_resolved",
                serde_json::json!({
                    "tool_call_id": tool_call_id,
                    "approval_id": record.approval_id,
                    "granted": granted,
                }),
            )
            .await;
            if !granted {
                return self.deny_tool(run_id, record, "denied by user").await;
            }
        }

        self.emit(
            run_id,
            "tool_started",
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "name": tool_name,
                "arguments": display_arguments,
            }),
        )
        .await;

        let ctx = ToolContext {
            run_id,
            workspace_id: request.workspace_id,
            root_path: request.root_path.clone(),
            additional_paths: request.additional_paths.clone(),
            approval_token,
        };
        // Mask credentials before the result reaches the transcript, the model
        // or the event log: one `cat .env` would otherwise copy a live key into
        // all three.
        // S2: `ask_user` blocks for the user's card answer. The tool itself only
        // validates and returns the card; the wait lives here so cancellation,
        // TTL, and `question_requested` / `question_resolved` events stay in one
        // place alongside the approval flow above.
        let mut outcome = registry
            .dispatch(ctx, &tool_name, arguments.clone())
            .await
            .map(|mut result| {
                mixd_tools::redact::redact_value(&mut result.content);
                if let Some(error) = result.error.as_mut() {
                    *error = mixd_tools::redact::redact_text(error);
                }
                result
            });

        if tool_name == "ask_user" {
            if let Ok(result) = &outcome {
                if result.ok {
                    let question_id = result
                        .content
                        .get("question_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if question_id.is_empty() {
                        // Never open a card the UI cannot resolve: an empty id
                        // would make `question_resolved` ambiguous and leave
                        // the run waiting on an answer nobody can route.
                        outcome = Ok(mixd_tools::ToolResult {
                            ok: false,
                            content: serde_json::Value::Null,
                            error: Some("ask_user returned no question id".to_string()),
                        });
                    } else {
                        // Reserve the next stamp atomically instead of `load`ing
                        // it. Every message queued from here on gets a seq
                        // strictly greater than this one, so an interjection
                        // sent in the gap before the card was announced cannot
                        // be mistaken for its answer.
                        let question_seq = self
                            .steering_seq
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        self.set_run_state(
                            run_id,
                            RunStatus::WaitingApproval,
                            serde_json::json!({ "questionId": question_id }),
                        )
                        .await;
                        self.emit(
                            run_id,
                            "question_requested",
                            serde_json::json!({
                                "tool_call_id": tool_call_id,
                                "question_id": question_id,
                                "question": result.content.get("question"),
                                "options": result.content.get("options"),
                                "multi_select": result.content.get("multi_select"),
                                "recommended": result.content.get("recommended"),
                                "timeout_secs": self.policy.ask_user_timeout_secs,
                            }),
                        )
                        .await;

                        // Wait on a per-run Notify rather than polling. Arm the
                        // waiter before reading the queue: `notify_waiters`
                        // only wakes registered waiters, so answering in the
                        // gap would otherwise stall until the timeout.
                        let cancellation = self.cancellation_of(run_id).await;
                        let notify = self.steering_notify(run_id).await;
                        let timeout =
                            std::time::Duration::from_secs(self.policy.ask_user_timeout_secs);
                        let mut answer: Option<String> = None;
                        loop {
                            if cancellation.is_cancelled() {
                                break;
                            }
                            let notified = notify.notified();
                            tokio::pin!(notified);
                            notified.as_mut().enable();
                            let mut pending = self.take_steering_after(run_id, question_seq).await;
                            if !pending.is_empty() {
                                answer = Some(pending.remove(0).text);
                                // Preserve any extra steering for the next turn.
                                self.requeue_steering(run_id, pending).await;
                                break;
                            }
                            tokio::select! {
                                biased;
                                _ = cancellation.cancelled_signal() => break,
                                _ = notified.as_mut() => continue,
                                _ = tokio::time::sleep(timeout) => break,
                            }
                        }
                        // A cancellation may have written a terminal status
                        // while the card was open. Only resume when the row is
                        // still in the state this question put it in.
                        self.set_run_state_if(
                            run_id,
                            RunStatus::WaitingApproval,
                            RunStatus::Running,
                            serde_json::json!({}),
                        )
                        .await;
                        match answer {
                            Some(text) => {
                                self.emit(
                                    run_id,
                                    "question_resolved",
                                    serde_json::json!({
                                        "tool_call_id": tool_call_id,
                                        "question_id": question_id,
                                        "accepted": true,
                                    }),
                                )
                                .await;
                                outcome = Ok(mixd_tools::ToolResult {
                                    ok: true,
                                    content: serde_json::json!({
                                        "kind": "ask_user_answer",
                                        "question_id": question_id,
                                        "answer": text,
                                    }),
                                    error: None,
                                });
                            }
                            None => {
                                let cancelled = cancellation.is_cancelled();
                                self.emit(
                                    run_id,
                                    "question_resolved",
                                    serde_json::json!({
                                        "tool_call_id": tool_call_id,
                                        "question_id": question_id,
                                        "accepted": false,
                                    }),
                                )
                                .await;
                                outcome = Ok(mixd_tools::ToolResult {
                                    ok: false,
                                    content: serde_json::Value::Null,
                                    error: Some(if cancelled {
                                        "question cancelled".to_string()
                                    } else {
                                        "question timed out waiting for an answer".to_string()
                                    }),
                                });
                            }
                        }
                    }
                }
            }
        }

        match outcome {
            Ok(result) => {
                record.result = Some(result.content.clone());
                record.error = result.error.clone();
                let status = if result.ok { "succeeded" } else { "failed" };
                let err_text = result.error.clone();
                let _ = self
                    .executor
                    .record_tool_call(
                        run_id,
                        &tool_call_id,
                        &tool_name,
                        // Persist the same masked arguments the UI and the
                        // event log see. The raw object above is still what
                        // reaches the Tool itself; only the transcript, the
                        // backup and the history API get the redacted copy.
                        display_arguments.clone(),
                        status,
                        Some(result),
                        err_text.as_deref(),
                    )
                    .await;
            }
            Err(err) => {
                let message = mixd_tools::redact::redact_text(&err.to_string());
                record.error = Some(message.clone());
                let _ = self
                    .executor
                    .record_tool_call(
                        run_id,
                        &tool_call_id,
                        &tool_name,
                        display_arguments.clone(),
                        "failed",
                        None,
                        Some(&message),
                    )
                    .await;
            }
        }

        self.emit(
            run_id,
            "tool_result",
            serde_json::json!({
                "tool_call_id": record.id,
                "name": record.name,
                "arguments": record.arguments,
                "result": record.result,
                "error": record.error,
            }),
        )
        .await;

        record
    }

    /// Look up the cancellation handle for a Run, if it is still active.
    pub(super) async fn cancellation_of(&self, run_id: RunId) -> RunCancellation {
        // The handle is removed on completion, so fall back to a standalone
        // (never-notified) handle rather than blocking forever.
        match self.active.lock().await.get(&run_id) {
            Some(c) => c.clone(),
            None => RunCancellation::new(),
        }
    }

    /// Record a refused Tool call and tell the model why, so it can adapt
    /// instead of retrying blindly.
    /// Report a call that the engine refused before it reached the Tool. Same
    /// bookkeeping as `deny_tool`, but recorded as a failure rather than a
    /// permission denial.
    pub(super) async fn fail_tool(
        &self,
        run_id: RunId,
        record: RunToolCall,
        reason: &str,
    ) -> RunToolCall {
        let message = format!("Tool `{}` was not run: {}", record.name, reason);
        let _ = self
            .executor
            .record_tool_call(
                run_id,
                &record.id,
                &record.name.clone(),
                record.arguments.clone(),
                "failed",
                None,
                Some(&message),
            )
            .await;
        self.emit(
            run_id,
            "tool_result",
            serde_json::json!({
                "tool_call_id": record.id,
                "name": record.name,
                "arguments": record.arguments,
                "result": message,
                "error": message,
            }),
        )
        .await;
        RunToolCall {
            error: Some(message),
            ..record
        }
    }

    pub(super) async fn deny_tool(
        &self,
        run_id: RunId,
        record: RunToolCall,
        reason: &str,
    ) -> RunToolCall {
        let message = format!("Tool `{}` was not run: {}", record.name, reason);
        let _ = self
            .executor
            .record_tool_call(
                run_id,
                &record.id,
                &record.name.clone(),
                record.arguments.clone(),
                "denied",
                None,
                Some(&message),
            )
            .await;
        self.emit(
            run_id,
            "tool_denied",
            serde_json::json!({
                "tool_call_id": record.id,
                "name": record.name,
                "reason": reason,
            }),
        )
        .await;
        self.emit(
            run_id,
            "tool_result",
            serde_json::json!({
                "tool_call_id": record.id,
                "name": record.name,
                "arguments": record.arguments,
                "result": message,
                "error": message,
            }),
        )
        .await;
        RunToolCall {
            error: Some(message),
            ..record
        }
    }
}
