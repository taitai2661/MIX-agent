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
        for chunk in batch.chunks(MAX_CONCURRENT_READS) {
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
        let tool_name = call.function.name.clone();
        let tool_call_id = call.id.clone();

        let mut record = RunToolCall {
            id: tool_call_id.clone(),
            name: tool_name.clone(),
            arguments: arguments.clone(),
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
        let decision = self
            .executor
            .resolve_permission(&tool_name, &permission_ctx, default_decision)
            .await;
        if decision == mixd_tools::permission::PermissionDecision::Deny {
            return self
                .deny_tool(run_id, record, "permission denied by rule")
                .await;
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
                    arguments.clone(),
                    "awaiting_approval",
                    None,
                    None,
                )
                .await;
            let prompt = self
                .executor
                .request_approval(
                    run_id,
                    &tool_call_id,
                    &tool_name,
                    arguments.clone(),
                    Some(request.workspace_id),
                    pattern_target.clone(),
                )
                .await;
            let Ok((rx, expires_at)) = prompt else {
                return self
                    .deny_tool(run_id, record, "could not open approval prompt")
                    .await;
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
                    "arguments": arguments,
                    // The prompt auto-denies at this instant. Without the
                    // deadline the card looked answerable forever and the run
                    // silently continued as "denied" while the user was away.
                    "expires_at": expires_at.to_rfc3339(),
                }),
            )
            .await;

            // Wait for the user's answer, the prompt TTL, or cancellation.
            let cancellation = self.cancellation_of(run_id).await;
            let ttl = (expires_at - chrono::Utc::now())
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(0));
            let outcome = tokio::select! {
                biased;
                _ = cancellation.cancelled_signal() => ApprovalOutcome::Deny,
                Ok(outcome) = rx => outcome,
                _ = tokio::time::sleep(ttl) => ApprovalOutcome::Deny,
            };
            self.set_run_state(run_id, RunStatus::Running, serde_json::json!({}))
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
                "arguments": arguments,
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
        let outcome = registry
            .dispatch(ctx, &tool_name, arguments.clone())
            .await
            .map(|mut result| {
                mixd_tools::redact::redact_value(&mut result.content);
                if let Some(error) = result.error.as_mut() {
                    *error = mixd_tools::redact::redact_text(error);
                }
                result
            });

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
                        arguments,
                        status,
                        Some(result),
                        err_text.as_deref(),
                    )
                    .await;
            }
            Err(err) => {
                record.error = Some(err.to_string());
                let _ = self
                    .executor
                    .record_tool_call(
                        run_id,
                        &tool_call_id,
                        &tool_name,
                        arguments,
                        "failed",
                        None,
                        Some(&err.to_string()),
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
