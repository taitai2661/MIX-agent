//! Memory spec §3 lifecycle transitions: `supersede` / `dispute` /
//! `resolve_conflict` / `decay`.
//!
//! Nothing in this module deletes a row — spec §3: `superseded` / `disputed` /
//! `expired` / `archived` keep their `content` and their revisions and are
//! only taken out of recall — and every transition first writes the row's
//! pre-transition state to `memory_revisions` (§1 「改変のたびに 1 行
//! `memory_revisions` へ改訂を残す」).
//!
//! The Docker edition's `memory/runtime.py` (`supersede` / `dispute` /
//! `resolve_conflict`) and `memory/service.py::decay` are the counterpart
//! implementations; this file keeps the *behaviour* aligned with them and
//! with §3 without sharing a line of their code.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;

use crate::memory::{
    log_action_event, MemoryService, DECAY_FLOOR, DECAY_HALF_LIFE_DAYS, DECAY_MIN_DAYS,
};

/// What a settled conflict left behind (spec §3
/// `resolve_conflict`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictResolution {
    /// The adopted Memory, back to `active`.
    pub winner: String,
    /// The rejected Memory; neither row is deleted.
    pub loser: String,
    /// `superseded`, or `expired` when the resolution dismissed the pair
    /// outright (`dismiss` / `rejected` / `discard`).
    pub loser_state: String,
    /// The resolution string the caller passed through, echoed back.
    pub resolution: String,
}

/// Read one RFC 3339 timestamp column or `data` key, `None` when absent or
/// unreadable — an unreadable anchor means "cannot prove this row has been
/// idle", which the callers answer by leaving the row alone.
fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

impl MemoryService {
    /// Spec §3: retire `old_id` in favour of `new_id`. **Not** a delete.
    ///
    /// The replacement Memory is mandatory: §3 「置換側（`new_id`）が無い
    /// `supersede` は拒否する」. A caller that wants the row gone outright has
    /// [`MemoryService::delete`], which is what §3 calls `forget` — forgetting
    /// and superseding are different operations and neither one fakes the
    /// other.
    ///
    /// The old row survives as `superseded` with `superseded_by_id` pointing
    /// at the replacement, after a revision of its pre-transition state (§1:
    /// no change may lose history). A settled row is not a contested one, so
    /// any lingering `disputed_by_id` is cleared.
    pub async fn supersede(
        storage: &mixd_storage::Storage,
        old_id: &str,
        new_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let new_id = new_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "supersede requires the replacement memory (new_id); \
                     use MemoryService::delete to forget outright"
                )
            })?;
        if new_id == old_id {
            anyhow::bail!("a memory cannot supersede itself");
        }
        let old = Self::get(storage, old_id)
            .await
            .map_err(|_| anyhow::anyhow!("old memory not found: {old_id}"))?;
        Self::get(storage, new_id)
            .await
            .map_err(|_| anyhow::anyhow!("replacement memory not found: {new_id}"))?;

        let now = Utc::now();
        let mut tx = storage.pool().begin().await?;
        Self::push_revision(&mut tx, old_id, &old.title, &old.content, now).await?;
        sqlx::query(
            "UPDATE memories SET lifecycle_state = 'superseded', superseded_by_id = ?, \
             disputed_by_id = NULL WHERE id = ?",
        )
        .bind(new_id)
        .bind(old_id)
        .execute(&mut *tx)
        .await?;
        // §4 「二重管理にしない」: the edge follows the column, in this
        // transaction, so the mirror cannot be read half-written. Clearing
        // `disputed_by_id` above takes any `contradicts` edge with it too.
        crate::memory_links::sync_links(&mut tx, old_id).await?;
        tx.commit().await?;
        log_action_event(storage, None, 0, "SUPERSEDE", old_id, Some(new_id), "").await;
        Ok(())
    }

    /// Spec §3: mark `memory_id` disputed against `conflicting_id`.
    ///
    /// **Only the named row is marked.** It leaves recall while the Memory it
    /// contradicts keeps serving it — an unresolved conflict must never be
    /// able to hide the fact that was already stored. `disputed_by_id` names
    /// the other side so [`MemoryService::resolve_conflict`] can settle the
    /// pair later, and a revision of the pre-dispute state is written first
    /// (§1).
    pub async fn dispute(
        storage: &mixd_storage::Storage,
        memory_id: &str,
        conflicting_id: &str,
    ) -> anyhow::Result<()> {
        if memory_id == conflicting_id {
            anyhow::bail!("a memory cannot conflict with itself");
        }
        let row = Self::get(storage, memory_id)
            .await
            .map_err(|_| anyhow::anyhow!("memory not found: {memory_id}"))?;
        Self::get(storage, conflicting_id)
            .await
            .map_err(|_| anyhow::anyhow!("conflicting memory not found: {conflicting_id}"))?;

        let now = Utc::now();
        let mut tx = storage.pool().begin().await?;
        Self::push_revision(&mut tx, memory_id, &row.title, &row.content, now).await?;
        sqlx::query(
            "UPDATE memories SET lifecycle_state = 'disputed', disputed_by_id = ? WHERE id = ?",
        )
        .bind(conflicting_id)
        .bind(memory_id)
        .execute(&mut *tx)
        .await?;
        // §4: `kind=contradicts` is `disputed_by_id` rendered as an edge, with
        // `source` = the side the column names and `target` = the disputed row.
        crate::memory_links::sync_links(&mut tx, memory_id).await?;
        tx.commit().await?;
        log_action_event(
            storage,
            None,
            0,
            "DISPUTE",
            memory_id,
            Some(conflicting_id),
            "",
        )
        .await;
        Ok(())
    }

    /// Spec §3: settle a dispute without losing either side's history.
    ///
    /// `conflict_id` is the row [`MemoryService::dispute`] marked; its
    /// `disputed_by_id` names the other side, and `prefer` — when given —
    /// must be one of those two ids. With no preference **the challenger does
    /// not win**: the side the dispute points at, i.e. the Memory that was
    /// already stored and has kept serving recall, is adopted (the Docker
    /// edition resolves the same way).
    ///
    /// The adopted row goes back to `active`; the rejected one becomes
    /// `superseded` — `superseded_by_id` aimed at the winner, so §3's link
    /// explains why it is gone — or `expired` when `resolution` reads as a
    /// dismissal (`dismiss` / `dismissed` / `rejected` / `discard`). Both rows
    /// get a revision first, and neither is deleted.
    pub async fn resolve_conflict(
        storage: &mixd_storage::Storage,
        conflict_id: &str,
        resolution: &str,
        prefer: Option<&str>,
    ) -> anyhow::Result<ConflictResolution> {
        let other_id: Option<Option<String>> =
            sqlx::query_scalar("SELECT disputed_by_id FROM memories WHERE id = ?")
                .bind(conflict_id)
                .fetch_optional(storage.pool())
                .await?;
        let other_id = other_id
            .flatten()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow::anyhow!("memory is not disputed: {conflict_id}"))?;
        if let Some(prefer) = prefer {
            if prefer != conflict_id && prefer != other_id {
                anyhow::bail!("prefer must be one of the two conflicting memories");
            }
        }
        // No preference → the already-stored side (the one the dispute points
        // at) wins; naming the marked row is the only way to adopt it.
        let winner_id = if prefer == Some(conflict_id) {
            conflict_id.to_string()
        } else {
            other_id.clone()
        };
        let loser_id = if winner_id == conflict_id {
            other_id
        } else {
            conflict_id.to_string()
        };

        let winner = Self::get(storage, &winner_id)
            .await
            .map_err(|_| anyhow::anyhow!("memory not found: {winner_id}"))?;
        let loser = Self::get(storage, &loser_id)
            .await
            .map_err(|_| anyhow::anyhow!("memory not found: {loser_id}"))?;

        let dismissed = matches!(
            resolution.trim().to_ascii_lowercase().as_str(),
            "dismiss" | "dismissed" | "rejected" | "discard"
        );
        let loser_state = if dismissed { "expired" } else { "superseded" };
        let now = Utc::now();
        let mut tx = storage.pool().begin().await?;
        Self::push_revision(&mut tx, &winner_id, &winner.title, &winner.content, now).await?;
        Self::push_revision(&mut tx, &loser_id, &loser.title, &loser.content, now).await?;
        // Adopting a Memory right now is a human re-activation (§1): its
        // recency moves with the decision.
        sqlx::query(
            "UPDATE memories SET lifecycle_state = 'active', disputed_by_id = NULL, \
             updated_at = ? WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(&winner_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE memories SET lifecycle_state = ?, superseded_by_id = ?, \
             disputed_by_id = NULL WHERE id = ?",
        )
        .bind(loser_state)
        .bind(&winner_id)
        .bind(&loser_id)
        .execute(&mut *tx)
        .await?;
        // Both rows moved a §3 column, so both mirrors are rebuilt from them
        // before the transaction can be read: the `contradicts` edge dies with
        // the dispute, and the loser's new `superseded_by_id` becomes a
        // `supersedes` edge (§4).
        crate::memory_links::sync_links(&mut tx, &winner_id).await?;
        crate::memory_links::sync_links(&mut tx, &loser_id).await?;
        tx.commit().await?;
        log_action_event(
            storage,
            None,
            0,
            "RESOLVE",
            &loser_id,
            Some(&winner_id),
            resolution,
        )
        .await;
        Ok(ConflictResolution {
            winner: winner_id,
            loser: loser_id,
            loser_state: loser_state.to_string(),
            resolution: resolution.to_string(),
        })
    }

    /// Spec §3 daily decay: weaken what has not been re-activated, archive at
    /// the floor.
    ///
    /// The rules the spec states, each one load-bearing:
    ///
    /// * `pinned` rows are never touched — §3 exempts them outright;
    /// * `updated_at` is **never written**: it is the input to the decay, so
    ///   bumping it would reset the age being measured;
    /// * only rows recall could see are candidates (§2 `retrievable_lifecycles`),
    ///   so an already-archived row cannot stay on the pass forever;
    /// * values decay exponentially on the anchor of the last application —
    ///   `data.last_decay_at` when a decay has run, else `updated_at`, which
    ///   any reinforce or edit has already moved. The newer of the two wins,
    ///   and anchor and values are written together or not at all, so daily
    ///   runs compound instead of double-decaying a row;
    /// * rows idle for less than [`DECAY_MIN_DAYS`] (0.5 day) are left alone —
    ///   a daily job has nothing to say about a memory from an hour ago;
    /// * a value that would fall below [`DECAY_FLOOR`] (0.1) is clamped there
    ///   and the row is archived, which is what takes it out of recall.
    ///
    /// Scalar decay is a recomputation of §1's own fields, not an edit, so it
    /// writes no revision and no event — only the transition to `archived`
    /// logs one `DECAY` action (§1).
    ///
    /// Returns how many rows were weakened (archived transitions included).
    pub async fn decay(storage: &mixd_storage::Storage) -> anyhow::Result<usize> {
        let now = Utc::now();
        let query = format!(
            "SELECT id, data, salience, strength, created_at, updated_at FROM memories \
             WHERE {} AND NOT {} ORDER BY updated_at",
            crate::memory::retrievable_predicate(),
            crate::memory::pinned_predicate(),
        );
        let rows = sqlx::query(&query).fetch_all(storage.pool()).await?;
        let mut touched = 0usize;

        for row in rows {
            let Ok(id) = row.try_get::<String, _>("id") else {
                continue;
            };
            // A blob that is not an object could be hiding a `pinned` flag,
            // and §3 exempts pinned rows from decay: when the flag cannot be
            // read, the safe answer is to leave the row alone.
            let Ok(raw) = row.try_get::<String, _>("data") else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            if !data.is_object() || crate::memory::is_pinned(&data) {
                continue;
            }

            let last_decay = data
                .get("last_decay_at")
                .and_then(|value| value.as_str())
                .and_then(parse_ts);
            let updated = row
                .try_get::<String, _>("updated_at")
                .ok()
                .as_deref()
                .and_then(parse_ts);
            let created = row
                .try_get::<String, _>("created_at")
                .ok()
                .as_deref()
                .and_then(parse_ts);
            let Some(anchor) = [last_decay, updated]
                .into_iter()
                .flatten()
                .max()
                .or(created)
            else {
                continue;
            };
            let elapsed_days = (now - anchor).num_milliseconds().max(0) as f64 / 86_400_000.0;
            if elapsed_days < DECAY_MIN_DAYS {
                continue;
            }

            let strength = row.try_get::<f64, _>("strength").unwrap_or(0.6);
            let salience = row.try_get::<f64, _>("salience").unwrap_or(0.5);
            let factor = 0.5_f64.powf(elapsed_days / DECAY_HALF_LIFE_DAYS);
            let next_strength = strength * factor;
            let next_salience = salience * factor;
            let archived = next_strength <= DECAY_FLOOR || next_salience <= DECAY_FLOOR;
            if !archived && next_strength == strength && next_salience == salience {
                continue;
            }
            let stored_strength = next_strength.clamp(DECAY_FLOOR, 1.0);
            let stored_salience = next_salience.clamp(DECAY_FLOOR, 1.0);

            // Note what this UPDATE deliberately does *not* touch: the
            // `updated_at` column (§3 — it is the input), and nothing else in
            // `data` besides the decay anchor.
            sqlx::query(
                "UPDATE memories SET salience = ?, strength = ?, \
                 data = json_set(data, '$.last_decay_at', ?) WHERE id = ?",
            )
            .bind(stored_salience)
            .bind(stored_strength)
            .bind(now.to_rfc3339())
            .bind(&id)
            .execute(storage.pool())
            .await?;
            if archived {
                sqlx::query("UPDATE memories SET lifecycle_state = 'archived' WHERE id = ?")
                    .bind(&id)
                    .execute(storage.pool())
                    .await?;
                log_action_event(
                    storage,
                    None,
                    0,
                    "DECAY",
                    &id,
                    None,
                    &format!("archived after {elapsed_days:.0} days idle"),
                )
                .await;
            }
            touched += 1;
        }
        Ok(touched)
    }

    /// §1 `pinned`: hold this Memory.
    ///
    /// A held row is exempt from decay and always retrievable, whatever its
    /// lifecycle says (§3). The flag lives in `data` (§1.1), so this touches
    /// neither `updated_at` — holding a fact is not changing it, and recency
    /// must not move — nor `memory_revisions`.
    pub async fn set_pinned(
        storage: &mixd_storage::Storage,
        id: &str,
        pinned: bool,
    ) -> anyhow::Result<()> {
        let raw: Option<String> = sqlx::query_scalar("SELECT data FROM memories WHERE id = ?")
            .bind(id)
            .fetch_optional(storage.pool())
            .await?;
        let Some(raw) = raw else {
            anyhow::bail!("memory not found: {id}");
        };
        let mut data = serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null);
        if !data.is_object() {
            data = Value::Object(serde_json::Map::new());
        }
        data["pinned"] = Value::Bool(pinned);
        sqlx::query("UPDATE memories SET data = ? WHERE id = ?")
            .bind(data.to_string())
            .bind(id)
            .execute(storage.pool())
            .await?;
        Ok(())
    }
}
