//! macOS Spotlight integration.
//!
//! The service owns the durable queue (`spotlight_items`), the settings flags,
//! and the lifecycle of the background worker. `payload` turns database rows
//! into index items; `index` writes them through Core Spotlight; `route` turns
//! a click on a result back into an app navigation.
//!
//! On non-macOS the whole module compiles to a no-op: the commands and settings
//! surface are shared, but every write path reports "unsupported" and the UI
//! hides the tab.

pub mod index;
pub mod payload;
#[cfg(target_os = "macos")]
pub mod route;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use mixd_storage::Storage;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use self::index::IndexSink;
use self::payload::{IndexItem, SpotlightKind};
use crate::commands::settings::SettingsView;

const META_LAST_SYNC: &str = "last_sync";
const META_LAST_ERROR: &str = "last_error";
const META_CLEAR_PENDING: &str = "clear_pending";
const BATCH: i64 = 50;
const DEBOUNCE: Duration = Duration::from_secs(2);
const IDLE_TICK: Duration = Duration::from_secs(30);

/// A navigation target produced by a Spotlight result click.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotlightTarget {
    pub kind: SpotlightKind,
    pub id: String,
    /// Conversation mode, so the UI can choose the chat/agent surface without
    /// a second round trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// What the settings pane shows.
#[derive(Debug, Clone, Serialize)]
pub struct SpotlightStatus {
    pub supported: bool,
    pub reason: Option<String>,
    pub enabled: bool,
    pub body_indexed: bool,
    pub indexed_count: i64,
    pub pending_count: i64,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct Flags {
    enabled: bool,
    body: bool,
}

pub struct SpotlightService {
    storage: Arc<Storage>,
    sink: Arc<dyn IndexSink>,
    notify: tokio::sync::Notify,
    flags: Mutex<Flags>,
    pending: Mutex<Option<SpotlightTarget>>,
    route_error: Mutex<Option<String>>,
    started: AtomicBool,
    /// Serializes reconcile/rebuild/worker passes. A `tokio::Mutex` so the
    /// worker can await database and completion-handler bridges while holding
    /// it without blocking a runtime thread.
    busy: tokio::sync::Mutex<()>,
}

impl SpotlightService {
    pub async fn new(storage: Arc<Storage>) -> Arc<Self> {
        let service = Arc::new(Self {
            storage,
            sink: index::default_sink(),
            notify: tokio::sync::Notify::new(),
            flags: Mutex::new(Flags {
                enabled: false,
                body: false,
            }),
            pending: Mutex::new(None),
            route_error: Mutex::new(None),
            started: AtomicBool::new(false),
            busy: tokio::sync::Mutex::new(()),
        });
        // Seed the flags before the first command can fire, so a message sent
        // immediately after launch is queued instead of waiting for the
        // background reconcile five seconds later.
        service.refresh_flags().await;
        service
    }

    pub fn set_route_error(&self, reason: String) {
        tracing::warn!(%reason, "Spotlight click routing is unavailable");
        *self.route_error.lock().unwrap() = Some(reason);
    }

    fn flags(&self) -> Flags {
        *self.flags.lock().unwrap()
    }

    fn set_flags(&self, enabled: bool, body: bool) {
        *self.flags.lock().unwrap() = Flags { enabled, body };
    }

    /// Start the background worker. Safe to call more than once; only the
    /// first call wins.
    pub fn start(self: &Arc<Self>) {
        #[cfg(not(target_os = "macos"))]
        {
            self.set_flags(false, false);
            return;
        }
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            service.refresh_flags().await;
            // Let the window and the first Run start before competing for the
            // database or Core Spotlight's helper process.
            tokio::time::sleep(Duration::from_secs(5)).await;
            if service.flags().enabled {
                if let Err(err) = service.reconcile().await {
                    tracing::warn!(%err, "initial Spotlight reconcile failed");
                }
            } else if service.clear_is_pending().await {
                // A previous disable could not reach corespotlightd. Retry on
                // this launch even though indexing stays off.
                if let Err(err) = service.clear().await {
                    tracing::warn!(%err, "retrying Spotlight clear failed");
                }
            }

            let mut failures: u32 = 0;
            loop {
                tokio::select! {
                    _ = service.notify.notified() => {}
                    _ = tokio::time::sleep(IDLE_TICK) => {}
                }
                if !service.flags().enabled {
                    continue;
                }
                // Coalesce a burst of create/rename/run-finished events.
                tokio::time::sleep(DEBOUNCE).await;
                match service.process_pending().await {
                    Ok(()) => failures = 0,
                    Err(err) => {
                        failures = failures.saturating_add(1);
                        let delay = match failures {
                            1 => Duration::from_secs(5),
                            2 => Duration::from_secs(30),
                            _ => Duration::from_secs(300),
                        };
                        tracing::warn!(
                            %err,
                            attempt = failures,
                        "Spotlight index pass failed; backing off"
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        });
    }

    /// Re-read the persisted settings and apply the runtime flags. Called at
    /// startup; settings changes go through [`Self::apply_settings`] instead so
    /// a toggle can trigger a rebuild immediately.
    async fn refresh_flags(&self) {
        let settings = crate::commands::settings::read_settings(self.storage.pool())
            .await
            .unwrap_or_default();
        let availability = self.sink.available();
        let available = availability.is_ok();
        if settings.spotlight_enabled && !available && cfg!(target_os = "macos") {
            tracing::warn!(
                reason = availability.err().unwrap_or_default(),
                "Spotlight setting is enabled but this environment cannot index"
            );
        }
        self.set_flags(
            settings.spotlight_enabled && available,
            settings.spotlight_index_body,
        );
    }

    /// Apply a saved settings document. The settings row is already durable;
    /// this only reacts to transitions.
    pub fn apply_settings(self: &Arc<Self>, settings: &SettingsView) {
        let previous = self.flags();
        let enabled = settings.spotlight_enabled && self.sink.available().is_ok();
        self.set_flags(enabled, settings.spotlight_index_body);

        let service = self.clone();
        if previous.enabled && !enabled {
            tauri::async_runtime::spawn(async move {
                if let Err(err) = service.clear().await {
                    tracing::warn!(%err, "clearing the Spotlight index failed");
                }
            });
        } else if enabled && (!previous.enabled || previous.body != settings.spotlight_index_body) {
            tauri::async_runtime::spawn(async move {
                if let Err(err) = service.rebuild().await {
                    tracing::warn!(%err, "rebuilding the Spotlight index failed");
                }
            });
        }
    }

    /// Queue one entity. No-op while the feature is off.
    pub async fn schedule(self: &Arc<Self>, kind: SpotlightKind, source_id: &str) {
        if !self.flags().enabled {
            return;
        }
        let item_id = kind.item_id(source_id);
        if let Err(err) = self.set_state(&item_id, kind, source_id, "pending").await {
            tracing::warn!(%err, item = %item_id, "could not queue a Spotlight item");
            return;
        }
        self.notify.notify_one();
    }

    /// Queue a deletion. The row is kept until the OS index confirms the
    /// delete, so a failed call is retried on the next pass.
    pub async fn schedule_deleted(self: &Arc<Self>, kind: SpotlightKind, source_id: &str) {
        if !self.flags().enabled {
            return;
        }
        let item_id = kind.item_id(source_id);
        if let Err(err) = self.set_state(&item_id, kind, source_id, "deleted").await {
            tracing::warn!(%err, item = %item_id, "could not queue a Spotlight delete");
            return;
        }
        self.notify.notify_one();
    }

    /// A finished Run can change the title and the last assistant preview.
    pub async fn schedule_run(self: &Arc<Self>, run_id: &str) {
        if !self.flags().enabled {
            return;
        }
        let row: Result<Option<(String,)>, _> =
            sqlx::query_as("SELECT conversation_id FROM runs WHERE id = ?")
                .bind(run_id)
                .fetch_optional(self.storage.pool())
                .await;
        match row {
            Ok(Some((conversation_id,))) => {
                self.schedule(SpotlightKind::Conversation, &conversation_id)
                    .await;
            }
            Ok(None) => {}
            Err(err) => {
                tracing::debug!(%err, run = %run_id, "could not resolve a run's conversation")
            }
        }
    }

    async fn set_state(
        &self,
        item_id: &str,
        kind: SpotlightKind,
        source_id: &str,
        state: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO spotlight_items (item_id, domain, source_id, content_hash, state) \
             VALUES (?, ?, ?, '', ?) \
             ON CONFLICT(item_id) DO UPDATE SET \
                 domain = excluded.domain, \
                 source_id = excluded.source_id, \
                 content_hash = '', \
                 state = excluded.state, \
                 last_error = NULL",
        )
        .bind(item_id)
        .bind(kind.domain())
        .bind(source_id)
        .bind(state)
        .execute(self.storage.pool())
        .await?;
        Ok(())
    }

    /// Rebuild the queue from the database and mark disappeared sources for
    /// deletion. This is both the crash-recovery path and the periodic safety
    /// net against missed schedule calls.
    pub async fn reconcile(self: &Arc<Self>) -> anyhow::Result<()> {
        if !self.flags().enabled {
            return Ok(());
        }
        let _guard = self.busy.lock().await;
        self.reconcile_locked().await
    }

    async fn reconcile_locked(&self) -> anyhow::Result<()> {
        let body = self.flags().body;
        for kind in [
            SpotlightKind::Conversation,
            SpotlightKind::Project,
            SpotlightKind::Knowledge,
        ] {
            let ids = payload::source_ids(self.storage.pool(), kind).await?;
            let mut seen: HashSet<String> = HashSet::with_capacity(ids.len());
            for source_id in ids {
                let Some(item) =
                    payload::build_item(self.storage.pool(), kind, &source_id, body).await?
                else {
                    continue;
                };
                seen.insert(item.item_id.clone());
                self.upsert_item(&item).await?;
            }

            // Rows in this domain whose source disappeared. They stay in the
            // table until `process_pending` confirms the OS delete.
            let known: Vec<(String,)> = sqlx::query_as(
                "SELECT item_id FROM spotlight_items WHERE domain = ? AND state != 'deleted'",
            )
            .bind(kind.domain())
            .fetch_all(self.storage.pool())
            .await?;
            for (item_id,) in known {
                if !seen.contains(&item_id) {
                    sqlx::query("UPDATE spotlight_items SET state = 'deleted' WHERE item_id = ?")
                        .bind(item_id)
                        .execute(self.storage.pool())
                        .await?;
                }
            }
        }
        self.notify.notify_one();
        Ok(())
    }

    async fn upsert_item(&self, item: &IndexItem) -> anyhow::Result<()> {
        let hash = item.content_hash();
        sqlx::query(
            "INSERT INTO spotlight_items \
                 (item_id, domain, source_id, content_hash, state) \
             VALUES (?, ?, ?, ?, 'pending') \
             ON CONFLICT(item_id) DO UPDATE SET \
                 domain = excluded.domain, \
                 source_id = excluded.source_id, \
                 content_hash = excluded.content_hash, \
                 state = CASE \
                     WHEN spotlight_items.state = 'indexed' \
                          AND spotlight_items.content_hash = excluded.content_hash \
                     THEN 'indexed' ELSE 'pending' END, \
                 last_error = NULL",
        )
        .bind(&item.item_id)
        .bind(item.domain)
        .bind(&item.source_id)
        .bind(hash)
        .execute(self.storage.pool())
        .await?;
        Ok(())
    }

    /// Index and delete queued rows in batches. Returns on the first failure;
    /// the remaining rows stay pending for the next pass.
    pub async fn process_pending(self: &Arc<Self>) -> anyhow::Result<()> {
        let _guard = self.busy.lock().await;
        loop {
            if !self.flags().enabled {
                return Ok(());
            }
            let body = self.flags().body;
            let pending: Vec<(String, String, String, String)> = sqlx::query_as(
                "SELECT item_id, domain, source_id, content_hash FROM spotlight_items \
                 WHERE state = 'pending' ORDER BY rowid LIMIT ?",
            )
            .bind(BATCH)
            .fetch_all(self.storage.pool())
            .await?;
            if pending.is_empty() {
                // Deletions are a separate queue; draining them here keeps a
                // delete from waiting for the next create/rename event.
                self.process_deletions(&self.sink).await?;
                self.clear_last_error().await?;
                return Ok(());
            }

            let mut items: Vec<(IndexItem, String)> = Vec::new();
            let mut missing: Vec<(String, String)> = Vec::new();
            for (item_id, domain, source_id, old_hash) in pending {
                let Some(kind) = kind_from_domain(&domain) else {
                    missing.push((item_id, old_hash));
                    continue;
                };
                match payload::build_item(self.storage.pool(), kind, &source_id, body).await {
                    Ok(Some(item)) => items.push((item, old_hash)),
                    Ok(None) => missing.push((item_id, old_hash)),
                    Err(err) => {
                        self.record_error(&err.to_string()).await?;
                        return Err(err);
                    }
                }
            }

            if !items.is_empty() {
                let sink = self.sink.clone();
                let to_index: Vec<IndexItem> = items.iter().map(|(item, _)| item.clone()).collect();
                let result = tokio::task::spawn_blocking(move || sink.index(&to_index)).await?;
                match result {
                    Ok(()) => {
                        let now = Utc::now().to_rfc3339();
                        for (item, old_hash) in &items {
                            // Conditional on the hash observed before indexing:
                            // a schedule() that landed while Core Spotlight was
                            // working cleared it, and that newer change must stay
                            // pending instead of being overwritten as indexed.
                            sqlx::query(
                                "UPDATE spotlight_items \
                                 SET state = 'indexed', content_hash = ?, indexed_at = ?, \
                                     last_error = NULL \
                                 WHERE item_id = ? AND state = 'pending' \
                                   AND content_hash = ?",
                            )
                            .bind(item.content_hash())
                            .bind(&now)
                            .bind(&item.item_id)
                            .bind(old_hash)
                            .execute(self.storage.pool())
                            .await?;
                        }
                        self.set_meta(META_LAST_SYNC, &now).await?;
                        self.clear_last_error().await?;
                    }
                    Err(err) => {
                        self.record_error(&err.to_string()).await?;
                        return Err(err);
                    }
                }
            }

            if !missing.is_empty() {
                let sink = self.sink.clone();
                let to_delete: Vec<String> =
                    missing.iter().map(|(item_id, _)| item_id.clone()).collect();
                let result = tokio::task::spawn_blocking(move || sink.delete(&to_delete)).await?;
                if let Err(err) = result {
                    self.record_error(&err.to_string()).await?;
                    return Err(err);
                }
                for (item_id, old_hash) in &missing {
                    sqlx::query(
                        "DELETE FROM spotlight_items \
                         WHERE item_id = ? AND state = 'pending' AND content_hash = ?",
                    )
                    .bind(item_id)
                    .bind(old_hash)
                    .execute(self.storage.pool())
                    .await?;
                }
            }

            self.process_deletions(&self.sink).await?;
            if items.is_empty() && missing.is_empty() {
                return Ok(());
            }
        }
    }

    async fn process_deletions(&self, sink: &Arc<dyn IndexSink>) -> anyhow::Result<()> {
        let deleted: Vec<(String, String)> = sqlx::query_as(
            "SELECT item_id, content_hash FROM spotlight_items WHERE state = 'deleted' \
             ORDER BY rowid LIMIT ?",
        )
        .bind(BATCH)
        .fetch_all(self.storage.pool())
        .await?;
        if deleted.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = deleted.iter().map(|(id, _)| id.clone()).collect();
        let sink = sink.clone();
        let to_delete = ids.clone();
        let result = tokio::task::spawn_blocking(move || sink.delete(&to_delete)).await?;
        result?;
        for (item_id, old_hash) in &deleted {
            sqlx::query(
                "DELETE FROM spotlight_items \
                 WHERE item_id = ? AND state = 'deleted' AND content_hash = ?",
            )
            .bind(item_id)
            .bind(old_hash)
            .execute(self.storage.pool())
            .await?;
        }
        Ok(())
    }

    /// Delete the whole app index and repopulate it. Used by the settings
    /// button and when indexing or body indexing is turned on.
    pub async fn rebuild(self: &Arc<Self>) -> anyhow::Result<()> {
        {
            let _guard = self.busy.lock().await;
            // Clear the queue first: a schedule() that lands while the OS
            // delete is in flight must survive as a pending row rather than be
            // erased after the fact.
            sqlx::query("DELETE FROM spotlight_items")
                .execute(self.storage.pool())
                .await?;
            let sink = self.sink.clone();
            tokio::task::spawn_blocking(move || sink.delete_all()).await??;
            self.clear_last_error().await?;
        }
        self.reconcile().await
    }

    /// Clear the OS index and the queue. Leaves the user's setting untouched,
    /// so the next save with indexing enabled rebuilds it.
    pub async fn clear(self: &Arc<Self>) -> anyhow::Result<()> {
        self.set_meta(META_CLEAR_PENDING, "1").await?;
        {
            let _guard = self.busy.lock().await;
            // Drop the queue before the OS call for the same reason as
            // rebuild(): a concurrent schedule is a newer change and should
            // not be erased.
            sqlx::query("DELETE FROM spotlight_items")
                .execute(self.storage.pool())
                .await?;
            let sink = self.sink.clone();
            if let Err(err) = tokio::task::spawn_blocking(move || sink.delete_all()).await? {
                self.record_error(&err.to_string()).await?;
                return Err(err);
            }
            self.delete_meta(META_LAST_SYNC).await?;
            self.delete_meta(META_CLEAR_PENDING).await?;
            self.clear_last_error().await?;
        }
        Ok(())
    }

    async fn clear_is_pending(&self) -> bool {
        self.get_meta(META_CLEAR_PENDING)
            .await
            .map(|value| value == "1")
            .unwrap_or(false)
    }

    pub async fn status(&self) -> SpotlightStatus {
        let settings = crate::commands::settings::read_settings(self.storage.pool())
            .await
            .unwrap_or_default();
        let availability = self.sink.available();
        let route_error = self.route_error.lock().unwrap().clone();
        let supported = availability.is_ok() && route_error.is_none();
        let reason = availability
            .err()
            .or(route_error)
            .or_else(|| (!cfg!(target_os = "macos")).then(|| "macOS only".to_string()));
        let indexed_count = self.count("state = 'indexed'").await.unwrap_or_default();
        let pending_count = self
            .count("state IN ('pending', 'deleted')")
            .await
            .unwrap_or_default();
        SpotlightStatus {
            supported,
            reason,
            enabled: settings.spotlight_enabled,
            body_indexed: settings.spotlight_index_body,
            indexed_count,
            pending_count,
            last_sync: self.get_meta(META_LAST_SYNC).await,
            last_error: self.get_meta(META_LAST_ERROR).await,
        }
    }

    async fn count(&self, predicate: &str) -> anyhow::Result<i64> {
        // `predicate` is one of this module's constants, never user input.
        let sql = format!("SELECT COUNT(*) FROM spotlight_items WHERE {predicate}");
        Ok(sqlx::query_scalar(&sql)
            .fetch_one(self.storage.pool())
            .await?)
    }

    pub fn take_pending(&self) -> Option<SpotlightTarget> {
        self.pending.lock().unwrap().take()
    }

    /// Resolve a clicked Spotlight identifier. A stale identifier (the thread
    /// or document was deleted after indexing) is removed from the queue.
    pub async fn open_item(self: &Arc<Self>, app: &AppHandle, item_id: String) {
        let Some((kind, source_id)) = SpotlightKind::parse(&item_id) else {
            tracing::warn!(%item_id, "ignoring an unknown Spotlight identifier");
            return;
        };
        let target = match kind {
            SpotlightKind::Conversation => {
                let mode: Option<String> = sqlx::query_scalar(
                    "SELECT COALESCE(json_extract(data, '$.mode'), 'chat') \
                     FROM conversations WHERE id = ?",
                )
                .bind(&source_id)
                .fetch_optional(self.storage.pool())
                .await
                .ok()
                .flatten();
                mode.map(|mode| SpotlightTarget {
                    kind,
                    id: source_id.clone(),
                    mode: Some(mode),
                })
            }
            SpotlightKind::Project => {
                let exists: Option<(String,)> =
                    sqlx::query_as("SELECT id FROM projects WHERE id = ?")
                        .bind(&source_id)
                        .fetch_optional(self.storage.pool())
                        .await
                        .ok()
                        .flatten();
                exists.map(|(id,)| SpotlightTarget {
                    kind,
                    id,
                    mode: None,
                })
            }
            SpotlightKind::Knowledge => {
                let exists: Option<(String,)> = sqlx::query_as(
                    "SELECT json_extract(data, '$.docId') FROM knowledge \
                     WHERE json_extract(data, '$.docId') = ? LIMIT 1",
                )
                .bind(&source_id)
                .fetch_optional(self.storage.pool())
                .await
                .ok()
                .flatten();
                exists.map(|(id,)| SpotlightTarget {
                    kind,
                    id,
                    mode: None,
                })
            }
        };

        let Some(target) = target else {
            self.schedule_deleted(kind, &source_id).await;
            return;
        };

        *self.pending.lock().unwrap() = Some(target.clone());
        #[cfg(target_os = "macos")]
        route::show_main_window(app);
        #[cfg(not(target_os = "macos"))]
        let _ = app;

        if let Err(err) = app.emit("spotlight://open", &target) {
            tracing::warn!(%err, "could not deliver a Spotlight navigation event");
        }
    }

    async fn set_meta(&self, key: &str, value: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO spotlight_meta (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(self.storage.pool())
        .await?;
        Ok(())
    }

    async fn get_meta(&self, key: &str) -> Option<String> {
        sqlx::query_scalar("SELECT value FROM spotlight_meta WHERE key = ?")
            .bind(key)
            .fetch_optional(self.storage.pool())
            .await
            .ok()
            .flatten()
    }

    async fn delete_meta(&self, key: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM spotlight_meta WHERE key = ?")
            .bind(key)
            .execute(self.storage.pool())
            .await?;
        Ok(())
    }

    async fn record_error(&self, message: &str) -> anyhow::Result<()> {
        self.set_meta(META_LAST_ERROR, message).await
    }

    async fn clear_last_error(&self) -> anyhow::Result<()> {
        self.delete_meta(META_LAST_ERROR).await
    }
}

/// Start the worker and install the click route. Called once from Tauri setup.
pub fn install(app: &AppHandle, service: Arc<SpotlightService>) {
    service.start();
    #[cfg(target_os = "macos")]
    {
        let service_for_error = service.clone();
        if let Err(err) = route::install(app, service) {
            service_for_error.set_route_error(err);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, service);
    }
}

fn kind_from_domain(domain: &str) -> Option<SpotlightKind> {
    match domain {
        payload::DOMAIN_CONVERSATIONS => Some(SpotlightKind::Conversation),
        payload::DOMAIN_PROJECTS => Some(SpotlightKind::Project),
        payload::DOMAIN_KNOWLEDGE => Some(SpotlightKind::Knowledge),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_mapping_round_trips() {
        for kind in [
            SpotlightKind::Conversation,
            SpotlightKind::Project,
            SpotlightKind::Knowledge,
        ] {
            assert_eq!(kind_from_domain(kind.domain()), Some(kind));
        }
        assert_eq!(kind_from_domain("unknown"), None);
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingSink {
        indexed: StdMutex<Vec<Vec<IndexItem>>>,
        deleted: StdMutex<Vec<Vec<String>>>,
    }

    impl IndexSink for RecordingSink {
        fn available(&self) -> Result<(), String> {
            Ok(())
        }

        fn index(&self, items: &[IndexItem]) -> anyhow::Result<()> {
            self.indexed.lock().unwrap().push(items.to_vec());
            Ok(())
        }

        fn delete(&self, item_ids: &[String]) -> anyhow::Result<()> {
            self.deleted.lock().unwrap().push(item_ids.to_vec());
            Ok(())
        }

        fn delete_all(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    async fn service() -> (Arc<SpotlightService>, Arc<RecordingSink>) {
        let dir =
            std::env::temp_dir().join(format!("mixd-spotlight-test-{}", uuid::Uuid::new_v4()));
        let storage = Arc::new(Storage::open(&dir).await.expect("open temp storage"));
        let sink = Arc::new(RecordingSink::default());
        let service = Arc::new(SpotlightService {
            storage,
            sink: sink.clone(),
            notify: tokio::sync::Notify::new(),
            flags: StdMutex::new(Flags {
                enabled: true,
                body: false,
            }),
            pending: StdMutex::new(None),
            route_error: StdMutex::new(None),
            started: AtomicBool::new(false),
            busy: tokio::sync::Mutex::new(()),
        });
        (service, sink)
    }

    async fn insert_conversation(service: &SpotlightService) {
        let data = serde_json::json!({
            "title": "Spotlight queue test",
            "mode": "chat",
            "modelId": "test-model",
            "workspaceId": uuid::Uuid::new_v4().to_string(),
            "projectId": null,
            "updatedAt": Utc::now().to_rfc3339(),
            "state": "active",
        });
        sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES ('c1', 'local', ?)")
            .bind(data.to_string())
            .execute(service.storage.pool())
            .await
            .expect("conversation");
        let message = serde_json::json!({
            "role": "assistant",
            "content": "The queue can build an item",
        });
        sqlx::query(
            "INSERT INTO messages (id, owner_id, data, conversation_id) \
             VALUES ('m1', 'local', ?, 'c1')",
        )
        .bind(message.to_string())
        .execute(service.storage.pool())
        .await
        .expect("message");
    }

    #[tokio::test]
    async fn reconcile_indexes_and_removes_disappeared_rows() {
        let (service, sink) = service().await;
        insert_conversation(&service).await;

        service.reconcile().await.expect("reconcile");
        service.process_pending().await.expect("index pass");
        assert_eq!(sink.indexed.lock().unwrap().len(), 1);

        sqlx::query("DELETE FROM messages WHERE conversation_id = 'c1'")
            .execute(service.storage.pool())
            .await
            .expect("delete messages");
        sqlx::query("DELETE FROM conversations WHERE id = 'c1'")
            .execute(service.storage.pool())
            .await
            .expect("delete conversation");

        service.reconcile().await.expect("reconcile delete");
        service.process_pending().await.expect("delete pass");
        assert_eq!(
            *sink.deleted.lock().unwrap(),
            vec![vec!["conversation:c1".to_string()]]
        );
    }

    #[tokio::test]
    async fn schedule_indexes_and_deletes_a_conversation() {
        let (service, sink) = service().await;
        insert_conversation(&service).await;

        service.schedule(SpotlightKind::Conversation, "c1").await;
        service.process_pending().await.expect("index pass");

        {
            let indexed = sink.indexed.lock().unwrap();
            assert_eq!(indexed.len(), 1);
            assert_eq!(indexed[0][0].item_id, "conversation:c1");
            assert_eq!(indexed[0][0].title, "Spotlight queue test");
        }
        let state: (String,) =
            sqlx::query_as("SELECT state FROM spotlight_items WHERE item_id = 'conversation:c1'")
                .fetch_one(service.storage.pool())
                .await
                .expect("queue row");
        assert_eq!(state.0, "indexed");

        service
            .schedule_deleted(SpotlightKind::Conversation, "c1")
            .await;
        service.process_pending().await.expect("delete pass");
        {
            let deleted = sink.deleted.lock().unwrap();
            assert_eq!(*deleted, vec![vec!["conversation:c1".to_string()]]);
        }
        let remaining: Option<(String,)> =
            sqlx::query_as("SELECT item_id FROM spotlight_items WHERE item_id = 'conversation:c1'")
                .fetch_optional(service.storage.pool())
                .await
                .expect("queue row");
        assert!(remaining.is_none());
    }
}
