use super::*;

impl RunEngine {
    pub fn subscribe(&self) -> broadcast::Receiver<RunEvent> {
        self.events.subscribe()
    }

    /// Publish an event to live subscribers and to the replay log.
    ///
    /// Milestone events (`status`, `tool_*`, `todo_list`, `error`, `budget_*`,
    /// …) are written straight through. Streamed token events (`text`,
    /// `reasoning`) are buffered and flushed in batches: they are the ones that
    /// arrive once per token, and serialising a SQLite INSERT behind each of
    /// them made the database the bottleneck of a streaming turn.
    pub async fn emit(&self, run_id: RunId, kind: &str, payload: serde_json::Value) {
        let sequence = self.next_sequence(run_id).await;
        let evt = RunEvent {
            run_id,
            sequence,
            kind: kind.to_string(),
            data: payload,
            created_at: chrono::Utc::now(),
        };
        // Cloning the event is the only cost of publishing it, and it is pure
        // waste when nothing is listening — scheduled and resumed runs have no
        // subscriber at all. A receiver attached later cannot see a message
        // published before it subscribed, so skipping the send is not a
        // behavioural change.
        if self.events.receiver_count() > 0 {
            if let Err(err) = self.events.send(evt.clone()) {
                // Only reachable on a receiver-count race; a dropped event
                // here is still durable via the replay log below.
                tracing::debug!(%run_id, kind, sequence, %err, "broadcast send had no live receiver");
            }
        }
        if STREAM_KINDS.contains(&kind) {
            self.buffer_event(evt).await;
            return;
        }
        // A Run's terminal state is itself a milestone, so draining the token
        // buffer here is what guarantees the streamed text is durable the
        // moment a Run completes, fails or is cancelled — before the row that
        // says so is written.
        self.flush_events(run_id).await;
        if let Err(err) = self.persist_event(&evt).await {
            tracing::error!(%run_id, sequence, kind, %err, "persisting run event failed");
        }
    }

    /// Park a token-stream event until the next batched flush.
    pub(super) async fn buffer_event(&self, evt: RunEvent) {
        let run_id = evt.run_id;
        let overflow = {
            let mut buffer = self.event_buffer.lock().await;
            let slot = buffer.entry(run_id).or_default();
            slot.push(evt);
            slot.len() >= EVENT_FLUSH_BATCH
        };
        if overflow {
            self.flush_events(run_id).await;
            return;
        }
        // Arm at most one timer per buffering window. Disarming happens in the
        // task *before* it drains, so an event buffered in that gap arms a new
        // timer instead of being left with no flush scheduled behind it.
        let mut scheduled = self.flush_scheduled.lock().await;
        if scheduled.insert(run_id) {
            let engine = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(EVENT_FLUSH_INTERVAL).await;
                engine.flush_scheduled.lock().await.remove(&run_id);
                engine.flush_events(run_id).await;
            });
        }
    }

    /// Write every buffered event of `run_id` to `run_events`, oldest first.
    pub async fn flush_events(&self, run_id: RunId) {
        let batch = {
            let mut buffer = self.event_buffer.lock().await;
            buffer.remove(&run_id).unwrap_or_default()
        };
        self.persist_batch(run_id, &batch).await;
    }

    /// Drain every Run's buffer. Used before a replay so reloading a
    /// conversation can never show a transcript the log has not caught up on.
    pub async fn flush_all_events(&self) {
        let all = {
            let mut buffer = self.event_buffer.lock().await;
            std::mem::take(&mut *buffer)
        };
        for (run_id, batch) in all {
            self.persist_batch(run_id, &batch).await;
        }
    }

    /// Reserve the next sequence number for a Run.
    ///
    /// The lock only ever covers a counter increment. The database lookup that
    /// seeds the counter (a Run resumed after a restart has to continue past
    /// the rows already stored) happens outside it: holding it across the
    /// SQLite round trip would serialise every TextDelta of a streaming turn
    /// behind one query, which is exactly the bottleneck this design removes.
    pub(super) async fn next_sequence(&self, run_id: RunId) -> i64 {
        {
            let mut seqs = self.sequences.lock().await;
            if let Some(entry) = seqs.get_mut(&run_id) {
                *entry += 1;
                return *entry;
            }
        }
        let base = self.persisted_sequence_base(run_id).await;
        let mut seqs = self.sequences.lock().await;
        // Double-checked: another task may have seeded while the query ran.
        // If so, just increment the existing counter so every caller gets a
        // unique value under the lock; never overwrite a newer counter with a
        // stale base.
        if let Some(entry) = seqs.get_mut(&run_id) {
            *entry += 1;
            return *entry;
        }
        let next = base.saturating_add(1);
        seqs.insert(run_id, next);
        next
    }

    /// Highest sequence already stored for a Run, or 0 when there is none.
    ///
    /// Retried once because a transient "database is locked" here used to be
    /// silently treated as "no rows", which would then collide on insert.
    pub(super) async fn persisted_sequence_base(&self, run_id: RunId) -> i64 {
        let mut last: Option<sqlx::Error> = None;
        for _ in 0..2 {
            match sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(sequence), 0) FROM run_events WHERE run_id = ?",
            )
            .bind(run_id.to_string())
            .fetch_optional(self.storage.pool())
            .await
            {
                Ok(Some(base)) => return base,
                Ok(None) => return 0,
                Err(err) => last = Some(err),
            }
        }
        if let Some(err) = last {
            tracing::error!(%run_id, %err, "reading the run event high-water mark failed");
        }
        0
    }

    pub(super) async fn persist_event(&self, evt: &RunEvent) -> anyhow::Result<()> {
        if let Err(err) = self.insert_event(evt).await {
            if !is_unique_violation(&err) {
                return Err(err);
            }
            // Only reachable when the high-water mark read failed and the
            // counter was seeded too low. `run_events` has
            // UNIQUE(run_id, sequence) and replay orders by that column, so
            // the row is renumbered against a fresh read rather than dropped:
            // dropping it would punch a hole in the transcript. The broadcast
            // envelope keeps the sequence it was published with, which only
            // affects gap detection, never replay order.
            let base = self.persisted_sequence_base(evt.run_id).await;
            let renumbered = base + 1;
            {
                let mut seqs = self.sequences.lock().await;
                let entry = seqs.entry(evt.run_id).or_insert(renumbered);
                *entry = (*entry).max(renumbered);
            }
            let stored = RunEvent {
                sequence: renumbered,
                ..evt.clone()
            };
            tracing::warn!(
                run = %evt.run_id,
                sequence = evt.sequence,
                renumbered,
                "run event sequence collided with a persisted row; renumbering"
            );
            self.insert_event(&stored).await
        } else {
            Ok(())
        }
    }

    /// Persist a drained buffer with one statement rather than one round trip
    /// per event.
    ///
    /// Token deltas arrive in bursts of up to `EVENT_FLUSH_BATCH`, so writing
    /// them one at a time made SQLite the pacing item of a streaming turn. A
    /// multi-row `INSERT` is atomic: when it is rejected it has written
    /// nothing, so the per-row path — which renumbers on a sequence collision —
    /// can be retried without risking duplicates.
    pub(super) async fn persist_batch(&self, run_id: RunId, batch: &[RunEvent]) {
        if batch.is_empty() {
            return;
        }
        // A long answer is thousands of `text` deltas. Storing one row per
        // token made `run_events` grow without bound and made opening an old
        // run read (and re-render) every token; merging adjacent deltas keeps
        // the replay identical while cutting the row count by up to the flush
        // batch size. Structural events are never merged.
        let batch = coalesce_stream_events(batch);
        if let Err(err) = self.insert_events(&batch).await {
            tracing::warn!(
                %run_id,
                %err,
                "batched run event insert failed; falling back to row by row"
            );
            for evt in &batch {
                if let Err(err) = self.persist_event(evt).await {
                    tracing::error!(
                        run = %run_id,
                        sequence = evt.sequence,
                        kind = %evt.kind,
                        %err,
                        "persisting buffered run event failed"
                    );
                }
            }
        }
    }

    pub(super) async fn insert_events(&self, batch: &[RunEvent]) -> anyhow::Result<()> {
        for chunk in batch.chunks(EVENT_FLUSH_BATCH) {
            let mut sql =
                String::from("INSERT INTO run_events (id, run_id, sequence, kind, data) VALUES ");
            let mut separator = "";
            for _ in chunk {
                sql.push_str(separator);
                sql.push_str("(?, ?, ?, ?, ?)");
                separator = ", ";
            }
            let mut statement = sqlx::query(&sql);
            for evt in chunk {
                statement = statement
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(evt.run_id.to_string())
                    .bind(evt.sequence)
                    .bind(&evt.kind)
                    .bind(evt.data.to_string());
            }
            statement.execute(self.storage.pool()).await?;
        }
        Ok(())
    }

    pub(super) async fn insert_event(&self, evt: &RunEvent) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO run_events (id, run_id, sequence, kind, data) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(evt.run_id.to_string())
        .bind(evt.sequence)
        .bind(&evt.kind)
        .bind(evt.data.to_string())
        .execute(self.storage.pool())
        .await?;
        Ok(())
    }
}

/// Keep a merged stream row below this so one pathological turn cannot create
/// a single multi-megabyte JSON blob.
const MAX_COALESCED_STREAM_BYTES: usize = 64 * 1024;

/// Merge adjacent `text` / `reasoning` deltas into one persisted row.
///
/// The reducer appends `data.delta` strings in sequence order, so merging is
/// invisible on replay. Non-stream events and a change of kind always break the
/// run, which preserves the original ordering around tool calls.
fn coalesce_stream_events(batch: &[RunEvent]) -> Vec<RunEvent> {
    let mut out: Vec<RunEvent> = Vec::with_capacity(batch.len());
    for evt in batch {
        if matches!(evt.kind.as_str(), "text" | "reasoning") {
            if let Some(prev) = out.last_mut() {
                if prev.kind == evt.kind && prev.run_id == evt.run_id {
                    let before = prev
                        .data
                        .get("delta")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let next = evt
                        .data
                        .get("delta")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let merged_len = before.len() + next.len();
                    if merged_len <= MAX_COALESCED_STREAM_BYTES {
                        prev.data = serde_json::json!({
                            "delta": format!("{before}{next}")
                        });
                        continue;
                    }
                }
            }
        }
        out.push(evt.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn event(sequence: i64, kind: &str, delta: &str) -> RunEvent {
        RunEvent {
            run_id: Uuid::new_v4(),
            sequence,
            kind: kind.to_string(),
            data: serde_json::json!({ "delta": delta }),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn adjacent_stream_deltas_merge_and_keep_the_first_sequence() {
        let run = Uuid::new_v4();
        let mut first = event(1, "text", "Hello");
        first.run_id = run;
        let mut second = event(2, "text", " world");
        second.run_id = run;
        let merged = coalesce_stream_events(&[first, second]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sequence, 1);
        assert_eq!(merged[0].data["delta"], "Hello world");
    }

    #[test]
    fn a_structural_event_breaks_the_merge() {
        let run = Uuid::new_v4();
        let mut text = event(1, "text", "before");
        text.run_id = run;
        let mut tool = event(2, "tool_started", "");
        tool.run_id = run;
        tool.data = serde_json::json!({ "tool_call_id": "t1" });
        let mut after = event(3, "text", "after");
        after.run_id = run;
        let merged = coalesce_stream_events(&[text, tool, after]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].data["delta"], "before");
        assert_eq!(merged[2].data["delta"], "after");
    }

    #[test]
    fn different_stream_kinds_do_not_merge() {
        let run = Uuid::new_v4();
        let mut text = event(1, "text", "answer");
        text.run_id = run;
        let mut reasoning = event(2, "reasoning", "thought");
        reasoning.run_id = run;
        let merged = coalesce_stream_events(&[text, reasoning]);
        assert_eq!(merged.len(), 2);
    }
}
