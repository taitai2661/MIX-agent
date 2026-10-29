use super::helpers::*;
use super::*;

impl RunEngine {
    pub async fn list_providers(&self) -> Vec<ProviderHandle> {
        self.providers.list().await
    }

    /// Replace the Tool Registry (used after an MCP reconnect).
    pub async fn replace_tools(&self, tools: ToolRegistry) {
        *self.tools.write().await = tools;
    }

    /// Advertised Tool specs, including Browser and MCP families.
    pub async fn tool_specs(&self) -> Vec<mixd_providers::ToolSpec> {
        let registry = self.tools.read().await;
        crate::runs::tool_specs::map_tool_specs(&registry.specs())
    }

    /// Mode-filtered specs (Docker `allowed_modes` parity). Chat excludes
    /// delegation and planning tools; thinking excludes delegation only.
    ///
    /// Takes the [`Mode`] enum rather than a string: unknown mode strings are
    /// rejected at the caller's parse boundary (with a warning + chat fallback)
    /// instead of silently degrading inside the engine.
    pub async fn tool_specs_for_mode(&self, mode: Mode) -> Vec<mixd_providers::ToolSpec> {
        let registry = self.tools.read().await;
        crate::runs::tool_specs::map_tool_specs(&registry.specs_for_mode(mode.as_str()))
    }

    /// Snapshot `paths` before a mutating call runs.
    ///
    /// Only the first sighting of a path is recorded: reverting a Run restores
    /// the state it started from, not the state before its most recent edit.
    pub(super) async fn snapshot_paths(
        &self,
        run_id: RunId,
        root_path: Option<&str>,
        paths: &[String],
    ) {
        if paths.is_empty() {
            return;
        }
        let root = root_path
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok());
        let mut needed: Vec<std::path::PathBuf> = Vec::new();
        {
            let mut guard = self.snapshots.lock().await;
            // Keep the undo history for the most recent Runs only; an
            // unbounded map would pin every edited file for the process
            // lifetime.
            if guard.len() > MAX_SNAPSHOT_RUNS && !guard.contains_key(&run_id) {
                if let Some(oldest) = guard.keys().next().copied() {
                    guard.remove(&oldest);
                }
            }
            let entry = guard.entry(run_id).or_default();
            if entry.len() >= MAX_SNAPSHOT_FILES {
                return;
            }
            for path in paths {
                let resolved = resolve_snapshot_path(root.as_deref(), path);
                if entry.iter().any(|snapshot| snapshot.path == resolved)
                    || needed.contains(&resolved)
                {
                    continue;
                }
                needed.push(resolved);
            }
        }
        let mut captured: Vec<FileSnapshot> = Vec::with_capacity(needed.len());
        for path in needed {
            let previous = tokio::fs::read(&path).await.ok();
            captured.push(FileSnapshot { path, previous });
        }
        if captured.is_empty() {
            return;
        }
        self.snapshots
            .lock()
            .await
            .entry(run_id)
            .or_default()
            .extend(captured);
    }

    /// Why a write should be refused because the file changed under the model.
    ///
    /// Returns `None` for a path the Run never read: the policy is "do not edit
    /// stale content", not "must read before every edit", which would reject
    /// perfectly good one-shot edits (and every existing test) for no safety.
    pub(super) async fn stale_write_reason(
        &self,
        run_id: RunId,
        root: Option<&str>,
        tool: &str,
        args: &serde_json::Value,
    ) -> Option<String> {
        let action = args
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let mutates = match tool {
            "edit" => true,
            "files" => matches!(action, "write" | "append" | "delete" | "create"),
            _ => false,
        };
        if !mutates {
            return None;
        }
        let paths = mutated_paths(tool, args);
        if paths.is_empty() {
            return None;
        }
        // Clone the small map and drop the guard before touching the disk.
        let known = {
            let guard = self.read_fingerprints.lock().await;
            guard.get(&run_id).cloned().unwrap_or_default()
        };
        if known.is_empty() {
            return None;
        }
        let root = root.map(std::path::Path::new);
        for path in paths {
            let key = normalize_tool_path(root, &path);
            let Some(expected) = known.get(&key).copied() else {
                continue;
            };
            match tokio::fs::read(&key).await {
                Ok(bytes) => {
                    if mixd_tools::files::fingerprint_of(&bytes) != expected {
                        return Some(format!(
                            "`{path}` changed on disk after you read it. Re-read it and copy \
                             the anchor from the current content."
                        ));
                    }
                }
                Err(_) => {
                    return Some(format!("`{path}` was deleted after you read it."));
                }
            }
        }
        None
    }

    /// Restore every file the Run changed to its pre-Run content.
    ///
    /// Returns the number of files restored. Terminal commands are not covered:
    /// which files a shell command writes is not knowable without sandboxing
    /// the write, so only `edit` / `files` mutations are snapshotted.
    pub async fn revert_run(&self, run_id: RunId) -> anyhow::Result<usize> {
        let snapshots = self
            .snapshots
            .lock()
            .await
            .remove(&run_id)
            .unwrap_or_default();
        let mut restored = 0usize;
        for snapshot in snapshots.into_iter().rev() {
            match snapshot.previous {
                Some(bytes) => {
                    if let Some(parent) = snapshot.path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    tokio::fs::write(&snapshot.path, bytes).await?;
                }
                None => {
                    if tokio::fs::metadata(&snapshot.path).await.is_ok() {
                        tokio::fs::remove_file(&snapshot.path).await?;
                    }
                }
            }
            restored += 1;
        }
        Ok(restored)
    }

    /// Queue a user message for the next turn of an active Run.
    ///
    /// Returns `false` when the Run is no longer active, so the caller can fall
    /// back to starting a normal turn instead of dropping the message into a
    /// queue nobody will drain.
    pub async fn steer(&self, run_id: RunId, content: String) -> bool {
        let text = content.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let active = self.active.lock().await.contains_key(&run_id);
        if !active {
            return false;
        }
        self.steering
            .lock()
            .await
            .entry(run_id)
            .or_default()
            .push(text);
        true
    }

    pub(super) async fn take_steering(&self, run_id: RunId) -> Vec<String> {
        self.steering
            .lock()
            .await
            .remove(&run_id)
            .unwrap_or_default()
    }

    /// Enable or disable the OS sandbox for shells spawned after this call.
    /// Existing sessions keep the profile they were started with.
    pub async fn set_terminal_sandbox(&self, enabled: bool) {
        let registry = self.tools.read().await.clone();
        registry.set_terminal_sandbox(enabled);
    }

    pub async fn cancel(&self, run_id: RunId) {
        let map = self.active.lock().await;
        if let Some(handle) = map.get(&run_id) {
            handle.cancel();
        }
    }

    pub async fn run_state(&self, id: RunId) -> Option<RunView> {
        let row = sqlx::query("SELECT id, conversation_id, data FROM runs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.storage.pool())
            .await;
        match row {
            Ok(Some(row)) => run_view_from_row(&row),
            Ok(None) => None,
            Err(err) => {
                tracing::error!(?err, %id, "run_state query failed");
                None
            }
        }
    }

    /// All Runs of a conversation, oldest first — the persisted counterpart of
    /// the live event stream, used to rebuild tool activity for past turns.
    ///
    /// A query failure is logged with the conversation id and yields an empty
    /// list (the UI shows "no runs" rather than failing the whole view); a
    /// corrupt row is skipped with its own log in [`run_view_from_row`], and
    /// the skip count is warned here so silent drops stay visible.
    pub async fn list_runs(&self, conversation_id: uuid::Uuid) -> Vec<RunView> {
        let rows = sqlx::query(
            "SELECT id, conversation_id, data FROM runs \
             WHERE conversation_id = ? ORDER BY created_at ASC",
        )
        .bind(conversation_id.to_string())
        .fetch_all(self.storage.pool())
        .await
        .unwrap_or_else(|err| {
            tracing::error!(?err, %conversation_id, "list_runs query failed");
            Vec::new()
        });
        let mut skipped = 0usize;
        let mut views: Vec<RunView> = rows
            .iter()
            .filter_map(|row| {
                run_view_from_row(row).or_else(|| {
                    skipped += 1;
                    None
                })
            })
            .collect();
        if skipped > 0 {
            tracing::warn!(%conversation_id, skipped, "list_runs skipped corrupt run rows");
        }
        // One grouped query for the whole conversation instead of one per run.
        // A denied approval also writes a `tool_result` with an error, so the
        // count covers both failures and refusals.
        let failures: Vec<(String, i64)> = sqlx::query_as(
            "SELECT e.run_id, COUNT(*) FROM run_events e \
             JOIN runs r ON r.id = e.run_id \
             WHERE r.conversation_id = ? AND e.kind = 'tool_result' \
               AND json_extract(e.data, '$.error') IS NOT NULL \
             GROUP BY e.run_id",
        )
        .bind(conversation_id.to_string())
        .fetch_all(self.storage.pool())
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(?err, %conversation_id, "failed tool count query failed");
            Vec::new()
        });
        let by_run: std::collections::HashMap<String, u32> = failures
            .into_iter()
            .map(|(run_id, count)| (run_id, count as u32))
            .collect();
        for view in &mut views {
            if let Some(count) = by_run.get(&view.id.to_string()) {
                view.failed_tool_count = *count;
            }
        }
        views
    }

    /// Record a status transition for a Run, both in the denormalised `status`
    /// column (what the UI lists) and inside the `data` blob (what
    /// [`run_view_from_row`] reads).
    ///
    /// Every value is bound as a parameter: paths and JSON payloads were
    /// previously interpolated into the SQL text, which SQLite rejected, and
    /// the resulting error was discarded — so no transition was ever persisted.
    pub(super) async fn set_run_state(
        &self,
        run_id: RunId,
        status: RunStatus,
        extra: serde_json::Value,
    ) {
        let mut sql =
            String::from("UPDATE runs SET data = json_set(data, '$.status', ?, '$.updatedAt', ?");
        let mut binds: Vec<String> =
            vec![status.as_str().to_string(), chrono::Utc::now().to_rfc3339()];
        for (k, v) in extra.as_object().into_iter().flatten() {
            sql.push_str(", ?, json(?)");
            // SQLite JSON path syntax: keys with special chars are quoted as
            // $."key", and a literal double-quote inside the key is escaped
            // as "" (not \"). Backslashes need no escaping in paths.
            let safe_key = k.replace('"', "\"\"");
            binds.push(format!("$.\"{safe_key}\""));
            binds.push(v.to_string());
        }
        sql.push_str("), status = ? WHERE id = ?");

        let mut query = sqlx::query(&sql);
        for b in &binds {
            query = query.bind(b);
        }
        let result = query
            .bind(status.as_str())
            .bind(run_id.to_string())
            .execute(self.storage.pool())
            .await;
        match result {
            Ok(res) if res.rows_affected() == 0 => {
                tracing::error!(
                    %run_id,
                    status = status.as_str(),
                    "set_run_state matched no row; status was not persisted"
                );
            }
            Ok(_) => {}
            Err(err) => {
                tracing::error!(%run_id, status = status.as_str(), %err, "set_run_state failed");
            }
        }
    }

    /// Drop the cached skill and agent catalogues.
    ///
    /// Both are read once and reused for the process, so a profile the user just
    /// saved would otherwise stay invisible until the app was restarted.
    pub async fn invalidate_catalogues(&self) {
        // Clone under the guard, drop it, then await the catalogue locks so a
        // concurrent `replace_tools` is never queued behind them.
        let registry = self.tools.read().await.clone();
        registry.invalidate_catalogues().await;
    }

    /// Append one turn to the conversation transcript.
    ///
    /// The transcript is what a later turn's history is rebuilt from, so the
    /// engine writes it here rather than leaving it to the caller: the engine is
    /// the only place that knows the correct interleaving of assistant tool-call
    /// turns and their results. Persisting only the final assistant text — which
    /// is what the shell did — meant a conversation's second turn contained no
    /// trace of the first turn's tool use, so the model re-derived the same
    /// search and edit work from scratch.
    pub(super) async fn append_message(
        &self,
        conversation_id: uuid::Uuid,
        run_id: RunId,
        model_id: &str,
        message: &HistoryMessage,
    ) {
        self.append_message_with_reasoning(conversation_id, run_id, model_id, message, None)
            .await;
    }

    /// Append one turn, optionally carrying the model's reasoning text.
    ///
    /// Reasoning normally lives only in the live view. A cancelled Run is the
    /// exception: its final assistant turn is never rebuilt from `messages`
    /// (there is no next turn), so if the partial answer is not written here it
    /// is lost the moment the UI retires the live block. `list_messages` already
    /// reads `$.reasoning`, so the persisted row renders exactly like a
    /// completed one.
    pub(super) async fn append_message_with_reasoning(
        &self,
        conversation_id: uuid::Uuid,
        run_id: RunId,
        model_id: &str,
        message: &HistoryMessage,
        reasoning: Option<&str>,
    ) {
        let data = serde_json::json!({
            "role": message.role,
            "content": message.content,
            "reasoning": reasoning.unwrap_or(""),
            "modelId": model_id,
            "runId": run_id.to_string(),
            "toolCalls": message.tool_calls,
            "toolCallId": message.tool_call_id,
            "name": message.name,
            "isError": message.is_error,
            "kind": message.role,
            "createdAt": chrono::Utc::now().to_rfc3339(),
        });
        if let Err(err) = sqlx::query(
            "INSERT INTO messages (id, owner_id, data, conversation_id) VALUES (?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(conversation_id.to_string())
        .execute(self.storage.pool())
        .await
        {
            tracing::warn!(?err, "failed to persist transcript message");
        }
    }

    /// Queue post-response memory formation (memory spec §6 自動形成).
    ///
    /// Called once per finished Run, right after the final assistant turn is
    /// persisted — 「ユーザーの発言とアシスタントの応答が 1 セットになったあとに
    /// 1 回」 — and always on a background task so the response path never
    /// waits on the queue. A temporary turn deliberately skips it: an
    /// exchange that leaves no trace must not leave a memory either.
    pub(super) fn spawn_memory_extraction(
        &self,
        run_id: RunId,
        user_message: &str,
        assistant: &str,
    ) {
        let storage = self.storage.clone();
        let run_id = run_id.to_string();
        let user = user_message.to_string();
        let assistant = assistant.to_string();
        tokio::spawn(async move {
            match crate::memory_jobs::enqueue(&storage, &run_id, &user, &assistant).await {
                Ok(true) => tracing::debug!(%run_id, "memory extraction queued"),
                // No `runs` row (a sub-agent turn) or already queued: both
                // mean "no job for this id", which is the contract.
                Ok(false) => {}
                Err(err) => tracing::warn!(%run_id, %err, "queueing memory extraction failed"),
            }
        });
    }
}
