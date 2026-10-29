use super::helpers::*;
use super::*;

impl RunEngine {
    pub(super) async fn release_run(&self, run_id: RunId) {
        self.active.lock().await.remove(&run_id);
        self.sequences.lock().await.remove(&run_id);
        self.flush_scheduled.lock().await.remove(&run_id);
        // A message typed as the Run finished has no turn left to reach.
        self.steering.lock().await.remove(&run_id);
        self.read_fingerprints.lock().await.remove(&run_id);
    }

    /// Record a Run's terminal state and return its outcome.
    ///
    /// Every exit path of the agent loop ends here. The four things a finished
    /// Run has to do — persist the status, emit it (which also drains the
    /// token buffer) and drop the per-run handles — used to be spelled out five
    /// separate times, and a path that skipped `release_run` left its `active`
    /// entry behind for the lifetime of the process.
    pub(super) async fn finish_run(
        &self,
        run_id: RunId,
        status: RunStatus,
        state: serde_json::Value,
        payload: serde_json::Value,
        outcome: RunOutcome,
    ) -> anyhow::Result<RunOutcome> {
        self.set_run_state(run_id, status, state).await;
        self.emit(run_id, "status", payload).await;
        self.release_run(run_id).await;
        Ok(RunOutcome { status, ..outcome })
    }

    /// Shared terminal path for the three cancellation exits (pre-turn check,
    /// budget-wait cancel, mid-stream cancel). They used to spell the same
    /// `finish_run(Cancelled)` inline three times; a field added to one copy
    /// (e.g. the `reasoning` added to the completed path) was silently missing
    /// from the others.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn finish_cancelled(
        &self,
        run_id: RunId,
        conversation_id: uuid::Uuid,
        model_id: &str,
        text: String,
        reasoning: String,
        tool_calls: Vec<RunToolCall>,
        usage: Option<Usage>,
        finish_reason: Option<String>,
        steps: u32,
    ) -> anyhow::Result<RunOutcome> {
        // Persist what the model had produced before the user pressed Stop.
        // Cancelling used to discard the partial answer entirely: the live
        // block was retired 1.5s after the terminal event and no assistant row
        // was ever written, so the transcript fell back to the bare user turn
        // with no sign that anything had been generated. The Run row keeps
        // `cancelled`, which is what the transcript uses to render the marker.
        if !text.is_empty() || !reasoning.is_empty() {
            let message = Message::text("assistant", text.clone());
            self.append_message_with_reasoning(
                conversation_id,
                run_id,
                model_id,
                &message_to_history(&message),
                Some(&reasoning),
            )
            .await;
        }
        self.finish_run(
            run_id,
            RunStatus::Cancelled,
            serde_json::json!({}),
            serde_json::json!({ "status": "cancelled" }),
            RunOutcome {
                status: RunStatus::Cancelled,
                text,
                reasoning,
                tool_calls,
                usage,
                finish_reason,
                steps,
            },
        )
        .await
    }
}
