use super::*;

impl RunEngine {
    /// Ask the user for a budget extension. Returns `None` when the Run was
    /// cancelled while waiting, `Some(granted)` otherwise.
    pub(super) async fn await_budget_extension(
        &self,
        run_id: RunId,
        reason: &str,
        budget: &Budget,
    ) -> Option<bool> {
        // An answer is persisted in `runs.data.budgetDecision` and nothing ever
        // cleared it, so the *second* prompt of a Run — or a resumed one — was
        // answered instantly with the first prompt's verdict instead of asking.
        self.budget_decisions.lock().await.remove(&run_id);
        // If this write fails the stale decision stays on the row and the next
        // prompt is auto-answered with the previous verdict, so the failure has
        // to be visible rather than discarded.
        if let Err(err) = sqlx::query(
            "UPDATE runs SET data = json_remove(data, '$.budgetDecision') \
             WHERE id = ? AND data IS NOT NULL",
        )
        .bind(run_id.to_string())
        .execute(self.storage.pool())
        .await
        {
            tracing::error!(
                %run_id,
                %err,
                "clearing the persisted budget decision failed; a later prompt may be auto-answered"
            );
        }
        self.set_run_state(
            run_id,
            RunStatus::BudgetExtensionPending,
            serde_json::json!({}),
        )
        .await;
        self.emit(
            run_id,
            "budget_extension_requested",
            serde_json::json!({
                "reason": reason,
                "stepLimit": budget.step_limit,
                "toolCallLimit": budget.tool_call_limit,
                "wallClockMinutes": budget.wall_clock_minutes,
                "tokenLimit": budget.token_limit,
                "tokensUsed": budget.tokens_used,
                "grants": budget.grants_used + 1,
            }),
        )
        .await;
        // Wait for the UI to answer via `budget_extension`: woken immediately
        // by `resolve_budget`'s notify, with the DB row as a fallback for
        // answers persisted before a restart (polled on a slow cadence, not
        // the old 250 ms busy loop).
        let deadline =
            std::time::Instant::now() + Duration::from_secs(budget.extension_timeout_secs);
        let cancellation = self.cancellation_of(run_id).await;
        let granted = loop {
            // A cancelled Run must not sit here until the deadline and then be
            // written out as `completed`.
            if cancellation.is_cancelled() {
                break false;
            }
            if let Some(granted) = self.take_budget_decision(run_id).await {
                break granted;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let wait = remaining.min(BUDGET_DB_POLL_INTERVAL);
            tokio::select! {
                biased;
                _ = cancellation.cancelled_signal() => break false,
                _ = self.budget_notify.notified() => continue,
                _ = tokio::time::sleep(wait) => continue,
            }
        };
        self.budget_decisions.lock().await.remove(&run_id);
        if cancellation.is_cancelled() {
            return None;
        }
        Some(granted)
    }

    /// Record the user's answer to a pending budget-extension request.
    ///
    /// Returns an error when the Run is not (or no longer) waiting for one, so
    /// the UI never treats an ignored answer as accepted. `budget` carries the
    /// raised limits a grant applies; they are persisted so a later resume sees
    /// the same numbers.
    pub async fn resolve_budget(
        &self,
        run_id: RunId,
        granted: bool,
        budget: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let mut sql = if granted {
            String::from(
                "UPDATE runs \
                 SET data = json_set(data, '$.budgetDecision', json('true')",
            )
        } else {
            String::from(
                "UPDATE runs \
                 SET data = json_set(data, '$.budgetDecision', json('false')",
            )
        };
        let mut binds: Vec<String> = Vec::new();
        if granted {
            if let Some(budget) = &budget {
                sql.push_str(", '$.budget', json(?)");
                binds.push(budget.to_string());
            }
            sql.push_str("), status = 'running'");
        } else {
            sql.push_str("), status = 'completed'");
        }
        sql.push_str(" WHERE id = ? AND status = 'budget_extension_pending'");

        let mut query = sqlx::query(&sql);
        for b in &binds {
            query = query.bind(b);
        }
        let res = query
            .bind(run_id.to_string())
            .execute(self.storage.pool())
            .await
            .map_err(|e| format!("resolve_budget update for run {run_id} failed: {e:#}"))?;
        if res.rows_affected() == 0 {
            return Err(format!(
                "run {run_id} is not waiting for a budget extension"
            ));
        }
        self.budget_decisions.lock().await.insert(run_id, granted);
        // Wake the `await_budget_extension` loop immediately; the DB row it
        // also wrote stays as the audit/fallback copy.
        self.budget_notify.notify_waiters();
        Ok(())
    }

    pub(super) async fn take_budget_decision(&self, run_id: RunId) -> Option<bool> {
        if let Some(granted) = self.budget_decisions.lock().await.get(&run_id).copied() {
            return Some(granted);
        }
        // A query failure reads as "no answer yet" (the waiter keeps polling),
        // but it must be logged: treating a locked database as a missing answer
        // delayed every budget prompt by the full timeout.
        let row: Option<(String,)> =
            sqlx::query_as::<_, (String,)>("SELECT data FROM runs WHERE id = ?")
                .bind(run_id.to_string())
                .fetch_optional(self.storage.pool())
                .await
                .map_err(|err| {
                    tracing::warn!(%run_id, %err, "take_budget_decision query failed; treating as no answer");
                    err
                })
                .unwrap_or(None);
        let data: serde_json::Value = row
            .and_then(|(raw,)| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        data.get("budgetDecision").and_then(|v| v.as_bool())
    }
}
