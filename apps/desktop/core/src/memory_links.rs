//! Memory spec §4: `memory_links` — 関連・上書き・矛盾 (the knowledge graph).
//!
//! One row of `memory_links` is one **directed** edge between two Memories,
//! unique on `(owner_id, source_memory_id, target_memory_id, kind)` so writing
//! the same edge twice updates one row instead of adding a second.
//!
//! §4's direction rule is the one thing to keep straight: `target_memory_id`
//! is the row that *carries* the §3 column and `source_memory_id` is the
//! Memory that column *names*. So `kind=supersedes` reads "source supersedes
//! target", `kind=contradicts` reads "source contradicts target" — §4 argues
//! this from §1/§3 there, and `sync_links` is the only writer that ever
//! produces such an edge.
//!
//! **「二重管理にしない」, and §3 → §4 is one-way**: only a §3 transition may
//! move `superseded_by_id` / `disputed_by_id`, and every one of them —
//! supersede, dispute, resolve, revision restore, delete — calls
//! `sync_links` on the row it changed, *inside the same transaction*.
//! `sync_links` derives the edge set back out of the column, never the other
//! way round. [`MemoryService::link`] is therefore refused for those two kinds
//! even though both are §2 words, and writes §4's own edges only. That closes
//! the §4 → §3 door: there is no path left that stands a column up without the
//! `lifecycle_state` §3 pairs with it, so nothing can end up holding
//! `superseded_by_id` while §2's `retrievable_lifecycles` still return it.
//!
//! The Docker edition's counterpart is `mix_agent/memory/service.py`
//! (`link` / `sync_links` / `_coactivate` / `_expand_links` / `suggest_links`
//! / `embedding_neighbor_links`); this file shares no line of their code, only
//! the contract.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::memory::{is_retrievable, MemoryService, RELATION_KINDS};
use crate::retrieval::{row_embedding, suggest_targets, vector_score, MemoryHit, QueryTerms};

// ---------------------------------------------------------------------------
// Spec §4's constants.
// ---------------------------------------------------------------------------

/// §4's default `weight` (0..1), and the weight a `relates_to` edge gets when
/// nothing better is known.
pub const LINK_DEFAULT_WEIGHT: f64 = 0.2;
/// §4 `degree_limit`: how many edges out of **one seed** the walk follows.
pub const LINK_DEGREE_LIMIT: usize = 5;
/// §4's per-search cap on rows the walk may add.
pub const LINK_EXPANSION_LIMIT: usize = 8;
/// §4 source (1): how many links one extracted row proposes.
pub const LINK_SUGGEST_LIMIT: usize = 3;
/// §4 source (3): how many embedding neighbours one row keeps.
pub const LINK_NEIGHBOR_TOP_K: usize = 3;
/// §4 source (3): the similarity floor, on §5's normalised cosine.
pub const LINK_NEIGHBOR_MIN_SIMILARITY: f64 = 0.75;
/// §4 source (3)'s setting key — **既定オフ**, and §4 says so explicitly.
pub const LINK_NEIGHBOR_SETTING: &str = "memory_link_embedding_neighbors";

/// §4 source (3) ranks against at most this many of the owner's rows. The
/// setting ships off, so this bounds the one pass that walks the whole store.
const NEIGHBOR_POOL_LIMIT: i64 = 2000;

/// What [`crate::retrieval`] reports about §4's contribution to a recall
/// (spec §5.3 `association_expansion`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssociationExpansion {
    /// The **seed** the edge walked out of, never the edge's own
    /// `source_memory_id`: §4's expansion is anchored on what the query
    /// matched, so the field answers "which hit pulled this in?".
    pub source: String,
    /// The row §4 added.
    pub target: String,
    /// The edge's `weight`, which is also the factor the added row's score
    /// was scaled down by.
    pub activation: f64,
    /// §2 `relation_kinds` entry this edge was stored under.
    pub kind: String,
}

/// One row of `memory_links` (spec §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryLink {
    pub id: String,
    pub owner_id: String,
    pub source_memory_id: String,
    pub target_memory_id: String,
    pub kind: String,
    pub weight: f64,
    pub coactivation_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const LINK_COLUMNS: &str = "id, owner_id, source_memory_id, target_memory_id, kind, \
                            weight, coactivation_count, created_at, updated_at";

fn link_from_row(row: &sqlx::sqlite::SqliteRow) -> MemoryLink {
    let ts = |key: &str| {
        row.try_get::<String, _>(key)
            .ok()
            .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|stamp| stamp.with_timezone(&Utc))
            .unwrap_or_else(Utc::now)
    };
    MemoryLink {
        id: row.try_get("id").unwrap_or_default(),
        owner_id: row.try_get("owner_id").unwrap_or_default(),
        source_memory_id: row.try_get("source_memory_id").unwrap_or_default(),
        target_memory_id: row.try_get("target_memory_id").unwrap_or_default(),
        kind: row.try_get("kind").unwrap_or_default(),
        weight: row.try_get("weight").unwrap_or(LINK_DEFAULT_WEIGHT),
        coactivation_count: row.try_get("coactivation_count").unwrap_or(0),
        created_at: ts("created_at"),
        updated_at: ts("updated_at"),
    }
}

async fn read_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
) -> anyhow::Result<MemoryLink> {
    let row = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM memory_links WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(link_from_row(&row))
}

/// §4's idempotent upsert: insert the edge if it is not there, refresh it if
/// it is. Returns `(row id, was it new)`.
///
/// `increment` is coactivation's "+1 per repeat" (§4 source (2)); a weight
/// that still equals [`LINK_DEFAULT_WEIGHT`] never overwrites a stored one, so
/// a plain `link()` cannot demote an edge a similarity source sharpened.
async fn upsert_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source_id: &str,
    target_id: &str,
    kind: &str,
    weight: f64,
    increment: bool,
) -> anyhow::Result<(String, bool)> {
    let now = Utc::now().to_rfc3339();
    let weight = weight.clamp(0.0, 1.0);
    let existing: Option<(String, i64, f64)> = sqlx::query_as(
        "SELECT id, coactivation_count, weight FROM memory_links \
         WHERE owner_id = ? AND source_memory_id = ? AND target_memory_id = ? AND kind = ?",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(source_id)
    .bind(target_id)
    .bind(kind)
    .fetch_optional(&mut **tx)
    .await?;

    match existing {
        None => {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO memory_links \
                 (id, owner_id, source_memory_id, target_memory_id, kind, weight, \
                  coactivation_count, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(source_id)
            .bind(target_id)
            .bind(kind)
            .bind(weight)
            .bind(if increment { 1i64 } else { 0i64 })
            .bind(&now)
            .bind(&now)
            .execute(&mut **tx)
            .await?;
            Ok((id, true))
        }
        Some((id, count, stored_weight)) => {
            let count = if increment { count + 1 } else { count };
            let stored_weight = if (weight - LINK_DEFAULT_WEIGHT).abs() > f64::EPSILON {
                weight
            } else {
                stored_weight
            };
            sqlx::query(
                "UPDATE memory_links SET weight = ?, coactivation_count = ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(stored_weight)
            .bind(count)
            .bind(&now)
            .bind(&id)
            .execute(&mut **tx)
            .await?;
            Ok((id, false))
        }
    }
}

// ---------------------------------------------------------------------------
// §4: the mirror writer.
// ---------------------------------------------------------------------------

/// Re-derive every §4 edge around `memory_id` from that row's §3 columns.
///
/// Called inside the caller's transaction by every transition that moves
/// `superseded_by_id` / `disputed_by_id` — supersede, dispute, resolve,
/// revision restore, delete — because §4 forbids the half-updated state: a
/// column naming an edge that is not there, or an edge claiming a pointer the
/// row no longer holds.
///
/// A row that is gone is a no-op: its edges went with it (`ON DELETE CASCADE`)
/// and the columns that named it were cleared by [`MemoryService::delete`]
/// first.
pub(crate) async fn sync_links(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    memory_id: &str,
) -> anyhow::Result<()> {
    for (column, kind) in [
        ("superseded_by_id", "supersedes"),
        ("disputed_by_id", "contradicts"),
    ] {
        sync_kind(tx, memory_id, column, kind).await?;
    }
    Ok(())
}

/// `sync_links` for one (column, kind) pair — §4's "follow the column".
async fn sync_kind(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    memory_id: &str,
    column: &str,
    kind: &str,
) -> anyhow::Result<()> {
    let found: Option<(String, Option<String>)> = sqlx::query_as(&format!(
        "SELECT owner_id, {column} FROM memories WHERE id = ?"
    ))
    .bind(memory_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((owner_id, pointed)) = found else {
        return Ok(());
    };
    let source_id = pointed
        .filter(|id| !id.is_empty())
        .filter(|id| id != memory_id);

    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, source_memory_id FROM memory_links \
         WHERE owner_id = ? AND target_memory_id = ? AND kind = ?",
    )
    .bind(&owner_id)
    .bind(memory_id)
    .bind(kind)
    .fetch_all(&mut **tx)
    .await?;

    // The edge the column names survives as it is; every other edge of this
    // kind targeting the row is a stale mirror (the pointer moved, or it was
    // never written by this path).
    let keeper = source_id.as_ref().and_then(|wanted| {
        existing
            .iter()
            .find(|(_, source)| source == wanted)
            .map(|(id, _)| id.clone())
    });
    for (id, _) in &existing {
        if Some(id) == keeper.as_ref() {
            continue;
        }
        sqlx::query("DELETE FROM memory_links WHERE id = ?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }

    if let Some(source) = source_id.filter(|_| keeper.is_none()) {
        // Only a row that still exists and is ours may be named: the foreign
        // keys would reject anything else, and a rejected insert would take
        // the caller's whole transition down with it.
        let exists: Option<String> =
            sqlx::query_scalar("SELECT id FROM memories WHERE id = ? AND owner_id = ?")
                .bind(&source)
                .bind(&owner_id)
                .fetch_optional(&mut **tx)
                .await?;
        if exists.is_some() {
            upsert_link(tx, &source, memory_id, kind, LINK_DEFAULT_WEIGHT, false).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// §4: the public surface.
// ---------------------------------------------------------------------------

impl MemoryService {
    /// §4: write one of §4's own edges, or refresh the one already there.
    ///
    /// `kind` must be one of §2's `relation_kinds` — anything else is refused
    /// before it can reach the table — and then §4's two §3 mirror kinds
    /// (`supersedes` / `contradicts`) are refused on top of that. They *are*
    /// §2's words; they are simply not `link()`'s to write. A §3 transition is
    /// the only thing allowed to move `superseded_by_id` / `disputed_by_id`,
    /// and `sync_links` derives the mirror edge out of that column in the
    /// same transaction, so the edge exists if and only if the column does.
    ///
    /// Allowing §4 → §3 here would produce exactly the row §3's diagram says
    /// cannot exist: `superseded_by_id` standing while `lifecycle_state` sits
    /// on `established`, so §2's `retrievable_lifecycles` keep returning a
    /// Memory that §3 claims was replaced. Refusing the two kinds removes that
    /// entry route instead of documenting around it — what this function
    /// writes is §4's own edges: `relates_to`, `supports`, `derived_from`,
    /// `caused_by`.
    ///
    /// To record that one Memory replaced or contradicts another, run
    /// [`MemoryService::supersede`] / [`MemoryService::dispute`]; those set the
    /// column *and* the lifecycle, which is what makes the mirror honest.
    pub async fn link(
        storage: &mixd_storage::Storage,
        source_id: &str,
        target_id: &str,
        kind: &str,
    ) -> anyhow::Result<MemoryLink> {
        if !RELATION_KINDS.contains(&kind) {
            anyhow::bail!("unknown relation kind: {kind}");
        }
        if kind == "supersedes" || kind == "contradicts" {
            // §4: 「§3 の列を書きうるのは §3 の遷移だけ」. The vocabulary still
            // has these six words and other code still reads them; the refusal
            // belongs to this one writer, not to §2.
            anyhow::bail!(
                "relation kind {kind:?} is a §3 mirror: only a §3 transition may write \
                 superseded_by_id / disputed_by_id, and sync_links derives the edge from \
                 it — see memory-spec.md §4 「§3 の列を書きうるのは §3 の遷移だけ」. \
                 Call MemoryService::supersede / dispute, or pick one of §4's own kinds."
            );
        }
        if source_id == target_id {
            anyhow::bail!("a memory cannot link to itself");
        }
        for (label, id) in [("source", source_id), ("target", target_id)] {
            let owned: Option<String> =
                sqlx::query_scalar("SELECT id FROM memories WHERE id = ? AND owner_id = ?")
                    .bind(id)
                    .bind(mixd_storage::records::SINGLE_USER_ID)
                    .fetch_optional(storage.pool())
                    .await?;
            if owned.is_none() {
                anyhow::bail!("{label} memory not found");
            }
        }

        let mut tx = storage.pool().begin().await?;
        let (id, _) = upsert_link(
            &mut tx,
            source_id,
            target_id,
            kind,
            LINK_DEFAULT_WEIGHT,
            false,
        )
        .await?;
        let link = read_link(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(link)
    }

    /// §4's read side: every edge that touches `memory_id`, in **both**
    /// directions, best-weight first.
    ///
    /// The Docker edition serves this from `GET
    /// /memories/{key}/associations`; the route checks that the key is the
    /// caller's before querying, which is why a missing or foreign id fails
    /// here instead of coming back as an empty list — an id that is not yours
    /// is a 404, not "no associations".
    pub async fn associations_for(
        storage: &mixd_storage::Storage,
        memory_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryLink>> {
        let owned: Option<String> =
            sqlx::query_scalar("SELECT id FROM memories WHERE id = ? AND owner_id = ?")
                .bind(memory_id)
                .bind(mixd_storage::records::SINGLE_USER_ID)
                .fetch_optional(storage.pool())
                .await?;
        if owned.is_none() {
            anyhow::bail!("memory not found");
        }
        let rows = sqlx::query(&format!(
            "SELECT {LINK_COLUMNS} FROM memory_links \
             WHERE owner_id = ? AND (source_memory_id = ? OR target_memory_id = ?) \
             ORDER BY weight DESC, coactivation_count DESC, updated_at DESC \
             LIMIT ?"
        ))
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(memory_id)
        .bind(memory_id)
        .bind(limit.max(1) as i64)
        .fetch_all(storage.pool())
        .await?;
        Ok(rows.iter().map(link_from_row).collect())
    }
}

// ---------------------------------------------------------------------------
// §4: generation source (2) — coactivation.
// ---------------------------------------------------------------------------

/// §4 source (2): rows recalled together by one **explicit** search.
///
/// Pairwise over the hit list in rank order (so the direction is stable for a
/// given ranking), `kind=relates_to`, `+1` per repeat. Gated on `activate` so
/// context injection — which searches every turn without activating — cannot
/// fabricate links out of "these two were mentioned today"; that is the same
/// reason §3 keeps injection from reinforcing.
pub(crate) async fn coactivate(
    storage: &mixd_storage::Storage,
    ids: &[String],
) -> anyhow::Result<usize> {
    if ids.len() < 2 {
        return Ok(0);
    }
    let mut tx = storage.pool().begin().await?;
    let mut created = 0usize;
    for index in 0..ids.len() {
        for target_id in &ids[index + 1..] {
            let source_id = &ids[index];
            // A coactivation is symmetric, so the pair is *one* row however
            // the two came back this time: §5.2 breaks a tie on `updated_at`,
            // which the reinforcing above just moved, so the order can flip
            // between searches. An edge already stored the other way round is
            // the same fact — increment it rather than adding its mirror.
            let existing: Option<(String,)> = sqlx::query_as(
                "SELECT id FROM memory_links \
                 WHERE owner_id = ? AND kind = 'relates_to' \
                   AND ((source_memory_id = ? AND target_memory_id = ?) \
                     OR (source_memory_id = ? AND target_memory_id = ?))",
            )
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(source_id)
            .bind(target_id)
            .bind(target_id)
            .bind(source_id)
            .fetch_optional(&mut *tx)
            .await?;

            match existing {
                Some((id,)) => {
                    sqlx::query(
                        "UPDATE memory_links SET coactivation_count = coactivation_count + 1, \
                         updated_at = ? WHERE id = ?",
                    )
                    .bind(Utc::now().to_rfc3339())
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                }
                None => {
                    // `_upsert_link(..., increment=True)` on a fresh row: one
                    // count already, because this *is* the first coactivation.
                    let (_, is_new) = upsert_link(
                        &mut tx,
                        source_id,
                        target_id,
                        "relates_to",
                        LINK_DEFAULT_WEIGHT,
                        true,
                    )
                    .await?;
                    created += usize::from(is_new);
                }
            }
        }
    }
    tx.commit().await?;
    Ok(created)
}

// ---------------------------------------------------------------------------
// §4: the 1-hop expansion.
// ---------------------------------------------------------------------------

/// What one walk out of the seeds produced.
pub(crate) struct Expansion {
    /// The rows §4 appended, after the seeds, `selection_reason` set.
    pub added: Vec<MemoryHit>,
    /// The edges those rows came in on (spec §5.3 `association_expansion`).
    pub edges: Vec<AssociationExpansion>,
    /// Which cap stopped the walk: `degree_limit`, `expansion_limit`, or
    /// nothing when the walk ran out of edges first.
    pub cutoff: Option<String>,
}

/// §4: exactly **one** hop out of the seed rows — never two.
///
/// The seeds are the rows the query actually matched; a linked row is appended
/// *after* them with `selection_reason="リンクから"` and a score the seed's
/// score was scaled down by the edge `weight`, so it can neither outrank a
/// real hit nor be mistaken for one. Both §4 caps are honoured here:
/// `degree_limit` edges per seed, [`LINK_EXPANSION_LIMIT`] rows per search,
/// whichever comes first (`cutoff` says which one did).
///
/// §2's `retrievable_lifecycles` gate is applied **again** on the way in: a
/// link into a superseded or archived memory does not resurrect it, while a
/// pinned one stays reachable (§3 exempts pinned rows from every automatic
/// retirement).
pub(crate) async fn expand_links(
    storage: &mixd_storage::Storage,
    seeds: &[MemoryHit],
) -> anyhow::Result<Expansion> {
    let mut expansion = Expansion {
        added: Vec::new(),
        edges: Vec::new(),
        cutoff: None,
    };
    if seeds.is_empty() {
        return Ok(expansion);
    }
    let seed_ids: Vec<String> = seeds.iter().map(|hit| hit.id.clone()).collect();
    let seed_set: HashSet<&str> = seed_ids.iter().map(String::as_str).collect();
    let score_of: HashMap<&str, f64> = seeds
        .iter()
        .map(|hit| (hit.id.as_str(), hit.score))
        .collect();
    // One JSON array feeds both halves of the OR: `json_each` keeps the
    // placeholders fixed, so the statement stays a single prepared query
    // whatever the seed count is.
    let ids_json = serde_json::to_string(&seed_ids)?;
    let rows = sqlx::query(
        "SELECT source_memory_id, target_memory_id, kind, weight FROM memory_links \
         WHERE owner_id = ? \
           AND (source_memory_id IN (SELECT value FROM json_each(?)) \
             OR target_memory_id IN (SELECT value FROM json_each(?))) \
         ORDER BY weight DESC, coactivation_count DESC, updated_at DESC",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(&ids_json)
    .bind(&ids_json)
    .fetch_all(storage.pool())
    .await?;

    // `other_id -> (seed it was reached from, weight, kind)` in first-seen
    // order, which is the edge ordering above: the strongest edge into a row
    // is the one that speaks for it.
    let mut reached: Vec<(String, (String, f64, String))> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut degree: HashMap<String, usize> = HashMap::new();
    let mut degree_truncated = false;

    for row in &rows {
        let source: String = row.try_get("source_memory_id").unwrap_or_default();
        let target: String = row.try_get("target_memory_id").unwrap_or_default();
        let (seed_id, other_id) = if seed_set.contains(source.as_str()) {
            (source.clone(), target.clone())
        } else {
            (target.clone(), source.clone())
        };
        if seed_set.contains(other_id.as_str()) {
            continue; // seed ↔ seed: both rows are already in the result
        }
        let count = degree.entry(seed_id.clone()).or_insert(0);
        if *count >= LINK_DEGREE_LIMIT {
            degree_truncated = true; // §4's degree cap stops this seed here
            continue;
        }
        *count += 1;
        if !seen.insert(other_id.clone()) {
            continue;
        }
        let weight: f64 = row
            .try_get::<f64, _>("weight")
            .ok()
            .filter(|value| value.is_finite())
            .unwrap_or(LINK_DEFAULT_WEIGHT);
        let kind: String = row.try_get("kind").unwrap_or_else(|_| "relates_to".into());
        reached.push((other_id, (seed_id, weight, kind)));
    }

    for (other_id, (seed_id, weight, kind)) in reached {
        if expansion.added.len() >= LINK_EXPANSION_LIMIT {
            expansion.cutoff = Some("expansion_limit".to_string());
            break;
        }
        let row = sqlx::query(
            "SELECT data, role, scope, lifecycle_state FROM memories \
             WHERE id = ? AND owner_id = ?",
        )
        .bind(&other_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .fetch_optional(storage.pool())
        .await?;
        let Some(row) = row else { continue };
        let data: serde_json::Value = row
            .try_get::<String, _>("data")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let lifecycle: String = row.try_get("lifecycle_state").unwrap_or_default();
        if !is_retrievable(&lifecycle, &data) {
            continue; // §3's gate, re-applied: a link does not resurrect a row
        }
        let seed_score = score_of.get(seed_id.as_str()).copied().unwrap_or(0.0);
        // Four decimals, like every other relevance this edge produces: the
        // scale-down is what stops a linked row from outranking a row the
        // query actually matched.
        let score = (seed_score * weight * 10_000.0).round() / 10_000.0;
        expansion.added.push(MemoryHit {
            id: other_id.clone(),
            title: data
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            content: data
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            role: row.try_get("role").unwrap_or_else(|_| "fact".into()),
            scope: row.try_get("scope").unwrap_or_else(|_| "user".into()),
            score,
            selection_reason: Some("リンクから".to_string()),
        });
        expansion.edges.push(AssociationExpansion {
            source: seed_id,
            target: other_id,
            activation: weight,
            kind,
        });
    }

    if expansion.cutoff.is_none() && degree_truncated {
        expansion.cutoff = Some("degree_limit".to_string());
    }
    Ok(expansion)
}

// ---------------------------------------------------------------------------
// §4: generation source (1) — what an extraction would have proposed.
// ---------------------------------------------------------------------------

/// §4 source (1): the links an extraction *would* have proposed.
///
/// §6's prompt is a verbatim cross-edition contract — `scripts/check-memory-parity.py`
/// byte-compares it — so asking the model for a `links` key would mean
/// changing §6 and the other edition in lockstep, not adding a field. The
/// proposal is therefore derived *after* the rows land, from what §5 already
/// agrees on: the new row's terms against the owner's store, ranked by the
/// same scoring §4's retrieval uses, capped at [`LINK_SUGGEST_LIMIT`] per
/// memory and restricted to rows §3 still lets retrieval see.
///
/// Returns how many links were **new**; a second run over the same rows is a
/// no-op that reports `0`, because the upsert finds what it just wrote.
pub async fn suggest_links(
    storage: &mixd_storage::Storage,
    memory_ids: &[String],
) -> anyhow::Result<usize> {
    if memory_ids.is_empty() {
        return Ok(0);
    }
    let mut created = 0usize;
    let mut tx = storage.pool().begin().await?;
    for memory_id in memory_ids {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT json_extract(data, '$.title'), json_extract(data, '$.content') \
             FROM memories WHERE id = ? AND owner_id = ?",
        )
        .bind(memory_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((title, content)) = row else {
            continue;
        };
        let terms = QueryTerms::new(&format!(
            "{} {}",
            title.unwrap_or_default(),
            content.unwrap_or_default()
        ));
        if terms.is_empty() {
            continue;
        }
        for hit in suggest_targets(storage, &terms, memory_id, LINK_SUGGEST_LIMIT).await? {
            let (_, is_new) = upsert_link(
                &mut tx,
                memory_id,
                &hit.id,
                "relates_to",
                LINK_DEFAULT_WEIGHT,
                false,
            )
            .await?;
            created += usize::from(is_new);
        }
    }
    tx.commit().await?;
    Ok(created)
}

// ---------------------------------------------------------------------------
// §4: generation source (3) — embedding neighbours, off by default.
// ---------------------------------------------------------------------------

/// §4 source (3): §5 embedding neighbours — **既定オフ**.
///
/// Only [`embedding_neighbor_links_if_enabled`] calls this, which is how §4's
/// "既定オフ" is honoured: links made here grow with the square of the store,
/// and the spec marks this source off for exactly that reason. Rows without a
/// vector, and rows §3 would not retrieve, never become neighbours.
///
/// Returns how many links were new.
pub async fn embedding_neighbor_links(
    storage: &mixd_storage::Storage,
    memory_ids: &[String],
) -> anyhow::Result<usize> {
    if memory_ids.is_empty() {
        return Ok(0);
    }
    let pool: Vec<(String, serde_json::Value, String)> = sqlx::query_as(
        "SELECT id, data, lifecycle_state FROM memories WHERE owner_id = ? \
         ORDER BY updated_at DESC LIMIT ?",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(NEIGHBOR_POOL_LIMIT)
    .fetch_all(storage.pool())
    .await?
    .into_iter()
    .filter_map(|(id, raw, lifecycle): (String, String, String)| {
        let data: serde_json::Value = serde_json::from_str(&raw).ok()?;
        Some((id, data, lifecycle))
    })
    .collect();

    let mut created = 0usize;
    let mut tx = storage.pool().begin().await?;
    for memory_id in memory_ids {
        let Some((_, data, _)) = pool.iter().find(|(id, _, _)| id == memory_id) else {
            continue;
        };
        let Some(vector) = row_embedding(data) else {
            continue;
        };
        let mut scored: Vec<(f64, &String)> = Vec::new();
        for (id, other, lifecycle) in &pool {
            if id == memory_id || !is_retrievable(lifecycle, other) {
                continue;
            }
            let Some(other_vector) = row_embedding(other) else {
                continue;
            };
            // §4: the floor is on §5's *normalised* cosine, `(1 + cos) / 2`.
            let Some(similarity) = vector_score(&vector, &other_vector) else {
                continue;
            };
            if similarity < LINK_NEIGHBOR_MIN_SIMILARITY {
                continue;
            }
            scored.push((similarity, id));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        for (similarity, other_id) in scored.into_iter().take(LINK_NEIGHBOR_TOP_K) {
            // Never weaker than the default, and rounded the way §4's
            // reference rounds: a similarity of 1.0 is a whole unit of weight,
            // not 0.99999.
            let (_, is_new) = upsert_link(
                &mut tx,
                memory_id,
                other_id,
                "relates_to",
                similarity.max(LINK_DEFAULT_WEIGHT).round(),
                false,
            )
            .await?;
            created += usize::from(is_new);
        }
    }
    tx.commit().await?;
    Ok(created)
}

/// §4 source (3) behind its setting. Returns `0` without touching the store
/// when the key is absent, `false`, or anything unparseable — the setting
/// ships off, and a malformed row must not be read as an opt-in.
pub async fn embedding_neighbor_links_if_enabled(
    storage: &mixd_storage::Storage,
    memory_ids: &[String],
) -> anyhow::Result<usize> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT data FROM app_settings WHERE id = 'singleton'")
            .fetch_optional(storage.pool())
            .await
            .ok()
            .flatten();
    let Some(raw) = row else {
        return Ok(0);
    };
    let enabled = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|doc| setting_flag(doc.get(LINK_NEIGHBOR_SETTING)?))
        .unwrap_or(false);
    if !enabled {
        return Ok(0);
    }
    embedding_neighbor_links(storage, memory_ids).await
}

/// Read a setting as a boolean the way §4's reference does: `true` / `1` /
/// `"true"` / `"1"` / `"on"` / `"yes"` all count, everything else is off.
fn setting_flag(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(flag) => Some(*flag),
        serde_json::Value::Number(number) => number.as_f64().map(|value| value != 0.0),
        serde_json::Value::String(text) => Some(
            text == "1"
                || text.eq_ignore_ascii_case("true")
                || text.eq_ignore_ascii_case("on")
                || text.eq_ignore_ascii_case("yes"),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §4's caps are the contract, not a tuning knob: a later change to one
    /// of them is a spec change and has to arrive as one.
    #[test]
    fn the_link_constants_are_the_spec_values() {
        assert_eq!(LINK_DEFAULT_WEIGHT, 0.2);
        assert_eq!(LINK_DEGREE_LIMIT, 5);
        assert_eq!(LINK_EXPANSION_LIMIT, 8);
        assert_eq!(LINK_SUGGEST_LIMIT, 3);
        assert_eq!(LINK_NEIGHBOR_TOP_K, 3);
        assert_eq!(LINK_NEIGHBOR_MIN_SIMILARITY, 0.75);
        assert_eq!(LINK_NEIGHBOR_SETTING, "memory_link_embedding_neighbors");
    }

    /// §4 source (3) ships off, and a malformed value is not an opt-in.
    #[test]
    fn the_embedding_neighbour_setting_reads_conservatively() {
        assert_eq!(setting_flag(&serde_json::json!(false)), Some(false));
        assert_eq!(setting_flag(&serde_json::json!(true)), Some(true));
        assert_eq!(setting_flag(&serde_json::json!("on")), Some(true));
        assert_eq!(setting_flag(&serde_json::json!("off")), Some(false));
        assert_eq!(setting_flag(&serde_json::json!(0)), Some(false));
        assert_eq!(setting_flag(&serde_json::json!(2)), Some(true));
        // An object or a null says nothing about the flag; neither is "on".
        assert_eq!(setting_flag(&serde_json::json!({})), None);
        assert_eq!(setting_flag(&serde_json::Value::Null), None);
    }
}
