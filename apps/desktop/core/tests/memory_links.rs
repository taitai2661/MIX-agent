//! Spec §4: `memory_links` — 関連・上書き・矛盾 (the knowledge graph).
//!
//! Mirrors the Docker edition's `tests/integration/test_memory_links.py` case
//! for case, so the two suites describe the same behaviour with the same
//! evidence — plus `delete`, which §4's writer contract covers and §9's
//! `forget` path has to honour. Everything runs against a temporary SQLite
//! file and never touches the network. The contract of record is
//! `desktop/docs/memory-spec.md` §4.

use mixd_core::memory::{
    apply_extraction, ExtractedMemory, MemoryCreate, MemoryItem, MemoryService,
};
use mixd_core::memory_links::{
    embedding_neighbor_links_if_enabled, suggest_links, AssociationExpansion, MemoryLink,
    LINK_DEFAULT_WEIGHT, LINK_DEGREE_LIMIT, LINK_EXPANSION_LIMIT, LINK_NEIGHBOR_MIN_SIMILARITY,
    LINK_SUGGEST_LIMIT,
};
use mixd_core::retrieval::{retrieve_all, retrieve_memories_debug, RetrievalDebug};
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::Storage;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A temporary data directory that cleans itself up.
struct TempStore {
    storage: Storage,
    dir: std::path::PathBuf,
}

impl TempStore {
    async fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mixd-memory-links-{tag}-{}-{:?}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let storage = Storage::open(&dir).await.expect("open storage");
        Self { storage, dir }
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One retrievable Memory whose title and content are the same text, so a
/// single query word can find it and nothing else in the store does.
async fn create(storage: &Storage, text: &str) -> MemoryItem {
    MemoryService::create(
        storage,
        MemoryCreate {
            title: text.to_string(),
            content: text.to_string(),
            role: "fact".to_string(),
            scope: "user".to_string(),
            source_kind: "user".to_string(),
            task_id: None,
        },
    )
    .await
    .expect("create memory")
}

/// Every row of `memory_links`, in a stable order.
async fn all_links(storage: &Storage) -> Vec<MemoryLink> {
    let rows = sqlx::query(
        "SELECT id, owner_id, source_memory_id, target_memory_id, kind, weight, \
         coactivation_count, created_at, updated_at FROM memory_links \
         WHERE owner_id = ? ORDER BY kind, source_memory_id, target_memory_id",
    )
    .bind(SINGLE_USER_ID)
    .fetch_all(storage.pool())
    .await
    .expect("read memory_links");
    rows.iter()
        .map(|row| MemoryLink {
            id: row.get("id"),
            owner_id: row.get("owner_id"),
            source_memory_id: row.get("source_memory_id"),
            target_memory_id: row.get("target_memory_id"),
            kind: row.get("kind"),
            weight: row.get("weight"),
            coactivation_count: row.get("coactivation_count"),
            created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("created_at"))
                .expect("created_at")
                .with_timezone(&chrono::Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<String, _>("updated_at"))
                .expect("updated_at")
                .with_timezone(&chrono::Utc),
        })
        .collect()
}

/// How many rows `memory_links` holds — §4's constraint, counted.
async fn link_count(storage: &Storage) -> usize {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_links")
        .fetch_one(storage.pool())
        .await
        .expect("count memory_links");
    count as usize
}

/// One of the two §3 pointer columns, `NULL` read back as `None`.
async fn column(storage: &Storage, id: &str, column: &str) -> Option<String> {
    sqlx::query_scalar(&format!("SELECT {column} FROM memories WHERE id = ?"))
        .bind(id)
        .fetch_optional(storage.pool())
        .await
        .expect("read pointer column")
        .flatten()
}

/// The three §3 facts §4 must never move on its own: both pointer columns
/// (empty when `NULL`) plus the lifecycle. Used to prove that a *refused*
/// `link()` leaves a row exactly as it was.
async fn state_of(storage: &Storage, id: &str) -> (String, String, String) {
    let row = sqlx::query(
        "SELECT superseded_by_id, disputed_by_id, lifecycle_state FROM memories WHERE id = ?",
    )
    .bind(id)
    .fetch_one(storage.pool())
    .await
    .expect("memory row");
    (
        row.get::<Option<String>, _>("superseded_by_id")
            .unwrap_or_default(),
        row.get::<Option<String>, _>("disputed_by_id")
            .unwrap_or_default(),
        row.get::<String, _>("lifecycle_state"),
    )
}

async fn set_lifecycle(storage: &Storage, id: &str, state: &str) {
    sqlx::query("UPDATE memories SET lifecycle_state = ? WHERE id = ?")
        .bind(state)
        .bind(id)
        .execute(storage.pool())
        .await
        .expect("set lifecycle_state");
}

/// The revision of `memory_id` whose §3 pointers read exactly `superseded_by`
/// and `disputed_by` — `None` meaning "explicitly null, and not standing".
///
/// Identifying a snapshot by what it recorded is the only stable way: the
/// revision table is append-only and a restore adds another row, so an index
/// into it would shift under the test.
async fn revision_with_pointers(
    storage: &Storage,
    memory_id: &str,
    superseded_by: Option<&str>,
    disputed_by: Option<&str>,
) -> String {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, data FROM memory_revisions \
         WHERE json_extract(data, '$.memoryId') = ? ORDER BY created_at DESC",
    )
    .bind(memory_id)
    .fetch_all(storage.pool())
    .await
    .expect("revisions");
    for (id, raw) in rows {
        let data: Value = serde_json::from_str(&raw).expect("revision JSON");
        let found = |key: &str| match data.get(key) {
            Some(Value::String(id)) => Some(id.as_str()),
            _ => None,
        };
        if found("superseded_by_id") == superseded_by && found("disputed_by_id") == disputed_by {
            return id;
        }
    }
    panic!("no revision of {memory_id} with §3 pointers {superseded_by:?} / {disputed_by:?}");
}

/// §4's read side, with the §5.3 keys that explain how the hits were chosen.
async fn debug_search(storage: &Storage, query: &str) -> (Vec<String>, RetrievalDebug) {
    let (hits, debug) = retrieve_memories_debug(storage, query, 8, false)
        .await
        .expect("debug search");
    let ids = hits.iter().map(|hit| hit.id.clone()).collect();
    (ids, debug)
}

// ---------------------------------------------------------------------------
// §4: the constraint itself.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_unique_constraint_holds_at_the_database_and_in_the_upsert() {
    // §4: one row per (owner, source, target, kind) — asserted three ways.
    let store = TempStore::new("unique").await;
    let first = create(&store.storage, "一意制約の始点ノード").await;
    let second = create(&store.storage, "一意制約の終点ノード").await;

    // (1) the upsert is idempotent: same tuple, same row, same id.
    let opened = MemoryService::link(&store.storage, &second.id, &first.id, "relates_to")
        .await
        .expect("link");
    let again = MemoryService::link(&store.storage, &second.id, &first.id, "relates_to")
        .await
        .expect("link again");
    assert_eq!(opened.id, again.id, "the upsert must return the same row");
    assert_eq!(link_count(&store.storage).await, 1);
    assert!((opened.weight - LINK_DEFAULT_WEIGHT).abs() < f64::EPSILON);
    assert_eq!(opened.coactivation_count, 0);

    // A different kind is a different edge, so it does get a row of its own.
    MemoryService::link(&store.storage, &second.id, &first.id, "supports")
        .await
        .expect("supports");
    assert_eq!(link_count(&store.storage).await, 2);

    // (2) the rule lives in the index, not only in the code.
    let index: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master \
         WHERE type = 'index' AND name = 'memory_link_owner_source_target_kind_uq'",
    )
    .fetch_one(store.storage.pool())
    .await
    .expect("unique index");
    for part in ["owner_id", "source_memory_id", "target_memory_id", "kind"] {
        assert!(
            index.contains(part),
            "the unique index must name {part}: {index}"
        );
    }

    // (3) SQLite itself refuses a hand-written duplicate.
    let duplicate = sqlx::query(
        "INSERT INTO memory_links \
         (id, owner_id, source_memory_id, target_memory_id, kind, weight, \
          coactivation_count, created_at, updated_at) \
         VALUES ('raw-duplicate', ?, ?, ?, 'relates_to', 0.2, 0, \
                 '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
    )
    .bind(SINGLE_USER_ID)
    .bind(&second.id)
    .bind(&first.id)
    .execute(store.storage.pool())
    .await;
    assert!(
        duplicate.is_err(),
        "the unique index must refuse a hand-written duplicate"
    );
    assert_eq!(link_count(&store.storage).await, 2);
}

#[tokio::test]
async fn supersede_writes_the_column_and_its_mirror_with_the_same_direction() {
    // §4 + §3: `kind=supersedes` **is** `superseded_by_id`, source = replacement.
    let store = TempStore::new("supersede").await;
    let old = create(&store.storage, "古い説明は五段階で採点する").await;
    let new = create(&store.storage, "新しい説明は七段階で採点する").await;
    MemoryService::supersede(&store.storage, &old.id, Some(&new.id))
        .await
        .expect("supersede");

    // The §3 column §1 documents as "これを置き換えた Memory" ...
    assert_eq!(
        column(&store.storage, &old.id, "superseded_by_id")
            .await
            .as_deref(),
        Some(new.id.as_str())
    );
    // ... and one edge saying the same thing, in the direction that reads as
    // "source supersedes target": source = 新 id, target = 旧 id.
    let superseding = all_links(&store.storage).await;
    assert_eq!(superseding.len(), 1);
    assert_eq!(superseding[0].kind, "supersedes");
    assert_eq!(superseding[0].source_memory_id, new.id);
    assert_eq!(superseding[0].target_memory_id, old.id);

    // The replacement row carries no §3 pointer of its own: superseding is a
    // statement *about* the old row.
    assert_eq!(
        column(&store.storage, &new.id, "superseded_by_id").await,
        None
    );

    // §4 「二重管理にしない」: the mirror is only reachable through the §3
    // transition, so re-running that transition is what has to stay
    // idempotent — `sync_links` finds the row it already wrote.
    let mirror_id = superseding[0].id.clone();
    MemoryService::supersede(&store.storage, &old.id, Some(&new.id))
        .await
        .expect("supersede again");
    let again = all_links(&store.storage).await;
    assert_eq!(
        again.len(),
        1,
        "the mirror stays one row however often the transition runs"
    );
    assert_eq!(
        again[0].id, mirror_id,
        "the mirror and the re-run must be one row"
    );
    assert!((again[0].weight - LINK_DEFAULT_WEIGHT).abs() < f64::EPSILON);
    assert_eq!(again[0].coactivation_count, 0);
    assert_eq!(link_count(&store.storage).await, 1);

    // §4 「§3 の列を書きうるのは §3 の遷移だけ」: the upsert refuses the
    // mirror kind outright, and the refusal leaves both rows untouched.
    for mirror_kind in ["supersedes", "contradicts"] {
        let err = MemoryService::link(&store.storage, &new.id, &old.id, mirror_kind)
            .await
            .expect_err("link() must not write a §3 mirror");
        assert!(
            err.to_string().contains("§3"),
            "the refusal has to cite §4's rule: {err}"
        );
        assert_eq!(link_count(&store.storage).await, 1, "no edge was added");
    }

    // Restoring the pre-supersede revision clears the column *and* the edge.
    let pre = revision_with_pointers(&store.storage, &old.id, None, None).await;
    MemoryService::restore(&store.storage, &old.id, &pre)
        .await
        .expect("restore");
    assert_eq!(
        column(&store.storage, &old.id, "superseded_by_id").await,
        None
    );
    assert!(all_links(&store.storage).await.is_empty());
    assert_eq!(link_count(&store.storage).await, 0);
}

#[tokio::test]
async fn dispute_and_resolve_keep_the_edges_in_step_with_both_columns() {
    // §4 + §3: `kind=contradicts` **is** `disputed_by_id`, source = the side
    // the column names, target = the disputed row.
    let store = TempStore::new("dispute").await;
    let stored = create(&store.storage, "移動手段は自転車").await;
    let challenge = create(&store.storage, "移動手段は電車").await;
    MemoryService::dispute(&store.storage, &challenge.id, &stored.id)
        .await
        .expect("dispute");

    assert_eq!(
        column(&store.storage, &challenge.id, "disputed_by_id")
            .await
            .as_deref(),
        Some(stored.id.as_str())
    );
    // `stored` is the row *carrying* the §3 column? No — `challenge` is, and
    // that is exactly the asymmetry §4 pins down.
    assert_eq!(
        column(&store.storage, &stored.id, "disputed_by_id").await,
        None
    );

    let contradicting = all_links(&store.storage).await;
    assert_eq!(contradicting.len(), 1);
    assert_eq!(contradicting[0].kind, "contradicts");
    assert_eq!(contradicting[0].source_memory_id, stored.id);
    assert_eq!(contradicting[0].target_memory_id, challenge.id);

    // Settle it: the challenge is now superseded, the contradiction is gone.
    let resolution =
        MemoryService::resolve_conflict(&store.storage, &challenge.id, "resolved", None)
            .await
            .expect("resolve");
    assert_eq!(resolution.winner, stored.id);
    assert_eq!(resolution.loser, challenge.id);
    assert_eq!(resolution.loser_state, "superseded");

    assert_eq!(
        column(&store.storage, &stored.id, "disputed_by_id").await,
        None
    );
    assert_eq!(
        column(&store.storage, &challenge.id, "disputed_by_id").await,
        None
    );
    assert_eq!(
        column(&store.storage, &challenge.id, "superseded_by_id")
            .await
            .as_deref(),
        Some(stored.id.as_str())
    );
    assert_eq!(
        column(&store.storage, &stored.id, "superseded_by_id").await,
        None
    );

    let remaining = all_links(&store.storage).await;
    assert_eq!(remaining.len(), 1, "the dispute left no contradicts edge");
    assert_eq!(remaining[0].kind, "supersedes");
    assert_eq!(remaining[0].source_memory_id, stored.id);
    assert_eq!(remaining[0].target_memory_id, challenge.id);
}

#[tokio::test]
async fn revision_restore_follows_the_pointers_it_restores() {
    // §1's history is what §4 mirrors: a revision carries `lifecycle_state`
    // and both pointers, so restoring text also restores the graph.
    let store = TempStore::new("restore").await;
    let stored = create(&store.storage, "コーヒーは毎朝三杯").await;
    let challenge = create(&store.storage, "コーヒーは毎朝一杯").await;
    MemoryService::dispute(&store.storage, &stored.id, &challenge.id)
        .await
        .expect("dispute");

    let after_dispute = all_links(&store.storage).await;
    assert_eq!(after_dispute.len(), 1);
    assert_eq!(after_dispute[0].source_memory_id, challenge.id);
    assert_eq!(after_dispute[0].target_memory_id, stored.id);

    // Settle the dispute: this pushes a revision of the *disputed* state, so
    // both directions can be walked without inventing a snapshot.
    MemoryService::resolve_conflict(&store.storage, &stored.id, "resolved", None)
        .await
        .expect("resolve");

    // (a) back to before the dispute: the column and the edge both go.
    let pre = revision_with_pointers(&store.storage, &stored.id, None, None).await;
    MemoryService::restore(&store.storage, &stored.id, &pre)
        .await
        .expect("restore pre-dispute");
    assert_eq!(
        column(&store.storage, &stored.id, "disputed_by_id").await,
        None
    );
    assert_eq!(
        column(&store.storage, &stored.id, "superseded_by_id").await,
        None
    );
    assert!(all_links(&store.storage).await.is_empty());

    // (b) forward to the disputed snapshot: the column and the edge both come
    // back — the restore does not have to be told about §4 twice.
    let disputed =
        revision_with_pointers(&store.storage, &stored.id, None, Some(&challenge.id)).await;
    MemoryService::restore(&store.storage, &stored.id, &disputed)
        .await
        .expect("restore disputed");
    assert_eq!(
        column(&store.storage, &stored.id, "disputed_by_id")
            .await
            .as_deref(),
        Some(challenge.id.as_str())
    );
    let back = all_links(&store.storage).await;
    assert_eq!(back.len(), 1);
    assert_eq!(
        (
            back[0].source_memory_id.as_str(),
            back[0].target_memory_id.as_str(),
            back[0].kind.as_str()
        ),
        (challenge.id.as_str(), stored.id.as_str(), "contradicts")
    );
}

#[tokio::test]
async fn deleting_a_row_clears_the_pointers_that_named_it() {
    // §9 forbids a column naming a row that no longer exists, so `forget`
    // detaches every pointer at it and rebuilds those rows' mirrors.
    let store = TempStore::new("delete").await;
    let old = create(&store.storage, "削除される旧ノード").await;
    let replacement = create(&store.storage, "削除で孤児になった新ノード").await;
    MemoryService::supersede(&store.storage, &old.id, Some(&replacement.id))
        .await
        .expect("supersede");
    assert_eq!(link_count(&store.storage).await, 1);
    assert_eq!(
        column(&store.storage, &old.id, "superseded_by_id")
            .await
            .as_deref(),
        Some(replacement.id.as_str())
    );

    // Delete the row the column *names* — the survivor may not keep a pointer
    // at something that no longer exists.
    MemoryService::delete(&store.storage, &replacement.id)
        .await
        .expect("delete the replacement");
    assert_eq!(link_count(&store.storage).await, 0, "edges go with the row");
    assert_eq!(
        column(&store.storage, &old.id, "superseded_by_id").await,
        None
    );
    let survivor = MemoryService::get(&store.storage, &old.id)
        .await
        .expect("the survivor still exists");
    // §4 cleans the *pointer*; §3's state is history and stays as it was —
    // the row really was superseded, and the pointer being cleared does not
    // make that untrue.
    assert_eq!(survivor.lifecycle_state, "superseded");

    // The other column, and the row that *named* the deleted one.
    let disputed = create(&store.storage, "争いを仕掛けられたノード").await;
    let challenger = create(&store.storage, "消える争いの相手").await;
    MemoryService::dispute(&store.storage, &disputed.id, &challenger.id)
        .await
        .expect("dispute");
    assert_eq!(link_count(&store.storage).await, 1);
    assert_eq!(
        column(&store.storage, &disputed.id, "disputed_by_id")
            .await
            .as_deref(),
        Some(challenger.id.as_str())
    );

    MemoryService::delete(&store.storage, &challenger.id)
        .await
        .expect("delete the challenger");
    assert_eq!(link_count(&store.storage).await, 0);
    assert_eq!(
        column(&store.storage, &disputed.id, "disputed_by_id").await,
        None
    );
    let survivor = MemoryService::get(&store.storage, &disputed.id)
        .await
        .expect("the survivor still exists");
    assert_eq!(survivor.lifecycle_state, "disputed");
}

// ---------------------------------------------------------------------------
// §4: the 1-hop expansion.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expansion_follows_exactly_one_hop_and_says_so_in_debug() {
    let store = TempStore::new("hop").await;
    let seed = create(&store.storage, "起点語を持つ種ノード").await;
    let hop_one = create(&store.storage, "一歩先の隣接ノード").await;
    let hop_two = create(&store.storage, "二歩先の遠いノード").await;
    MemoryService::link(&store.storage, &seed.id, &hop_one.id, "relates_to")
        .await
        .expect("link");
    MemoryService::link(&store.storage, &hop_one.id, &hop_two.id, "relates_to")
        .await
        .expect("link");

    let (hits, debug) = retrieve_memories_debug(&store.storage, "起点語", 8, false)
        .await
        .expect("debug search");
    assert_eq!(
        hits.first().map(|hit| hit.id.as_str()),
        Some(seed.id.as_str())
    );

    // Exactly one hop: `hop_one` comes in on the seed's edge, `hop_two` does
    // not — following it would be two.
    assert!(
        hits.iter().any(|hit| hit.id == hop_one.id),
        "one hop must be followed: {hits:?}"
    );
    assert!(
        !hits.iter().any(|hit| hit.id == hop_two.id),
        "two hops must not be followed: {hits:?}"
    );

    let added = hits
        .iter()
        .find(|hit| hit.id == hop_one.id)
        .expect("hop_one present");
    assert_eq!(added.selection_reason.as_deref(), Some("リンクから"));
    assert!(
        added.score < hits[0].score,
        "a linked row may not outrank a row the query matched: {} vs {}",
        added.score,
        hits[0].score
    );

    assert_eq!(debug.seed_ids, vec![seed.id.clone()]);
    assert_eq!(debug.memory_ids, vec![seed.id.clone(), hop_one.id.clone()]);
    assert_eq!(debug.stop_reason, "depth");
    assert_eq!(debug.association_expansion_count, 1);
    assert_eq!(debug.association_cutoff, None);
    assert_eq!(
        debug.association_expansion,
        vec![AssociationExpansion {
            source: seed.id.clone(),
            target: hop_one.id.clone(),
            activation: LINK_DEFAULT_WEIGHT,
            kind: "relates_to".to_string(),
        }]
    );
}

#[tokio::test]
async fn the_degree_cap_stops_a_walk_before_the_expansion_cap_would() {
    // §4: `degree_limit = 5` per seed, `expansion_limit = 8` per search —
    // the smaller one has to win, and `association_cutoff` says which fired.
    assert_eq!(LINK_DEGREE_LIMIT, 5);
    assert_eq!(LINK_EXPANSION_LIMIT, 8);

    let store = TempStore::new("degree").await;
    let seed = create(&store.storage, "degreehub の種ノード").await;
    let mut neighbours = Vec::new();
    for index in 0..20 {
        let neighbour = create(
            &store.storage,
            &format!("neighbour{index} は素数を数えるノード"),
        )
        .await;
        MemoryService::link(&store.storage, &seed.id, &neighbour.id, "relates_to")
            .await
            .expect("link");
        neighbours.push(neighbour.id);
    }

    let (hits, debug) = debug_search(&store.storage, "degreehub").await;
    assert_eq!(debug.seed_ids, vec![seed.id.clone()]);
    assert_eq!(debug.association_expansion_count, LINK_DEGREE_LIMIT);
    assert_eq!(debug.association_cutoff.as_deref(), Some("degree_limit"));
    assert_eq!(debug.stop_reason, "depth");
    // Seed + five additions: the expansion cap of eight was never reached.
    assert_eq!(hits.len(), 1 + LINK_DEGREE_LIMIT);

    let seeds = [seed.id.as_str()];
    assert_eq!(debug.association_expansion.len(), LINK_DEGREE_LIMIT);
    for edge in &debug.association_expansion {
        assert_eq!(edge.source, seed.id, "every edge walks out of the seed");
        assert!(neighbours.contains(&edge.target), "…to a real neighbour");
    }
    assert!(debug
        .association_expansion
        .iter()
        .all(|edge| !seeds.contains(&edge.target.as_str())));
}

#[tokio::test]
async fn expansion_obeys_the_lifecycle_gate_and_keeps_a_pinned_row() {
    // §2's `retrievable_lifecycles` gate is applied *again* on the way in: a
    // link must not resurrect a retired row, while §3 exempts a pinned one
    // from every automatic retirement.
    let store = TempStore::new("lifecycle").await;
    let seed = create(&store.storage, "lifespanhub の種ノード").await;
    let active = create(&store.storage, "an active evaluation node").await;
    let retired = create(&store.storage, "a retired evaluation node").await;
    let archived = create(&store.storage, "an archived evaluation node").await;
    let pinned = create(&store.storage, "a pinned evaluation node").await;

    MemoryService::supersede(&store.storage, &retired.id, Some(&active.id))
        .await
        .expect("supersede");
    set_lifecycle(&store.storage, &archived.id, "archived").await;
    set_lifecycle(&store.storage, &pinned.id, "archived").await;
    MemoryService::set_pinned(&store.storage, &pinned.id, true)
        .await
        .expect("pin");

    for id in [&active.id, &retired.id, &archived.id, &pinned.id] {
        MemoryService::link(&store.storage, &seed.id, id, "relates_to")
            .await
            .expect("link");
    }

    let (hits, debug) = debug_search(&store.storage, "lifespanhub").await;
    let present: Vec<&str> = hits.iter().map(String::as_str).collect();
    assert!(present.contains(&seed.id.as_str()));
    assert!(present.contains(&active.id.as_str()), "a live row stays");
    assert!(
        present.contains(&pinned.id.as_str()),
        "§3 exempts a pinned row from the gate"
    );
    assert!(
        !present.contains(&retired.id.as_str()),
        "a superseded row stays out of recall"
    );
    assert!(
        !present.contains(&archived.id.as_str()),
        "an archived row stays out of recall"
    );
    assert_eq!(debug.association_expansion_count, 2);
    assert_eq!(debug.association_cutoff, None);
}

// ---------------------------------------------------------------------------
// §4: generation sources (1)–(3).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn coactivation_links_count_up_on_an_activating_search_only() {
    // §4 source (2): rows recalled together by one *explicit* search.
    let store = TempStore::new("coactivation").await;
    let first = create(&store.storage, "koherence の説明は一つ目").await;
    let second = create(&store.storage, "koherence の説明は二つ目").await;
    assert_eq!(link_count(&store.storage).await, 0);

    MemoryService::search(&store.storage, "koherence", 8)
        .await
        .expect("activating search");
    let pair = all_links(&store.storage).await;
    assert_eq!(pair.len(), 1);
    assert_eq!(pair[0].kind, "relates_to");
    assert_eq!(pair[0].coactivation_count, 1);
    let endpoints = [
        pair[0].source_memory_id.as_str(),
        pair[0].target_memory_id.as_str(),
    ];
    assert!(endpoints.contains(&first.id.as_str()) && endpoints.contains(&second.id.as_str()));

    // The same explicit search again increments, still one row (§4 制約).
    MemoryService::search(&store.storage, "koherence", 8)
        .await
        .expect("activating search again");
    assert_eq!(link_count(&store.storage).await, 1);
    assert_eq!(all_links(&store.storage).await[0].coactivation_count, 2);

    // §3: context injection searches every turn without activating — it must
    // neither reinforce nor invent links from "mentioned today".
    let before = link_count(&store.storage).await;
    let block = retrieve_all(&store.storage, "koherence")
        .await
        .expect("context injection");
    assert_eq!(link_count(&store.storage).await, before);
    assert_eq!(all_links(&store.storage).await[0].coactivation_count, 2);
    assert_eq!(block.memories.len(), 2, "injection still returns both rows");
    // §5.3: the keys name the seeds and the walk that found nothing to add.
    let debug = block.debug.expect("retrieve_all fills the debug keys");
    assert_eq!(debug.seed_ids.len(), 2);
    assert_eq!(debug.stop_reason, "flat");
    assert_eq!(debug.association_expansion_count, 0);
    assert_eq!(debug.association_cutoff, None);
}

#[tokio::test]
async fn link_rejects_a_kind_outside_section_2() {
    // §4: `kind` is validated *before* it can reach the column or the table.
    let store = TempStore::new("kind").await;
    let first = create(&store.storage, "語彙検査の始点ノード").await;
    let second = create(&store.storage, "語彙検査の終点ノード").await;

    for bad in ["friends", "SUPERSEDES", "supersedes ", ""] {
        let err = MemoryService::link(&store.storage, &second.id, &first.id, bad)
            .await
            .expect_err("an unknown kind must be refused");
        assert!(
            err.to_string().contains("unknown relation kind"),
            "a §2 kind has to be named in the error: {err}"
        );
    }
    assert_eq!(
        link_count(&store.storage).await,
        0,
        "nothing reached the table"
    );

    // §4 「§3 の列を書きうるのは §3 の遷移だけ」: both mirror kinds are in
    // §2's vocabulary, so they get past the §4 check — and are refused on top
    // of it. §4's whole argument is that allowing §4 → §3 would leave a row
    // holding `superseded_by_id` while `lifecycle_state` sat on `established`
    // (still inside `retrievable_lifecycles`), so what the refusal has to
    // prove is that neither row's §3 state moves a millimetre.
    let before_first = state_of(&store.storage, &first.id).await;
    let before_second = state_of(&store.storage, &second.id).await;
    let untouched = (String::new(), String::new(), "established".to_string());
    assert_eq!(before_first, untouched);
    assert_eq!(before_second, untouched);

    for mirror_kind in ["supersedes", "contradicts"] {
        for (source, target) in [(&second.id, &first.id), (&first.id, &second.id)] {
            let err = MemoryService::link(&store.storage, source, target, mirror_kind)
                .await
                .expect_err("link() must refuse a §3 mirror kind");
            let message = err.to_string();
            assert!(
                message.contains("§3") && message.contains("memory-spec.md"),
                "the refusal has to send the caller to §4's rule: {message}"
            );
        }
    }
    assert_eq!(
        state_of(&store.storage, &first.id).await,
        before_first,
        "a refused link() must not move first's superseded_by_id / disputed_by_id / lifecycle"
    );
    assert_eq!(
        state_of(&store.storage, &second.id).await,
        before_second,
        "a refused link() must not move second's superseded_by_id / disputed_by_id / lifecycle"
    );
    assert_eq!(
        link_count(&store.storage).await,
        0,
        "a refused kind writes no edge either"
    );

    // The same gate holds for an endpoint that is not yours (Desktop has one
    // owner, so: an id that is not there).
    let err = MemoryService::link(&store.storage, "no-such-memory", &first.id, "relates_to")
        .await
        .expect_err("a missing source must be refused");
    assert!(err.to_string().contains("source memory not found"));
    let err = MemoryService::link(&store.storage, &second.id, "no-such-memory", "relates_to")
        .await
        .expect_err("a missing target must be refused");
    assert!(err.to_string().contains("target memory not found"));
    let err = MemoryService::link(&store.storage, &first.id, &first.id, "relates_to")
        .await
        .expect_err("a self-link must be refused");
    assert!(err.to_string().contains("itself"));

    // Every §2 kind is accepted; the first one alone survives.
    assert!(
        MemoryService::link(&store.storage, &second.id, &first.id, "supports")
            .await
            .is_ok()
    );
    assert!(
        MemoryService::link(&store.storage, &second.id, &first.id, "derived_from")
            .await
            .is_ok()
    );
    assert!(
        MemoryService::link(&store.storage, &second.id, &first.id, "caused_by")
            .await
            .is_ok()
    );
    assert_eq!(link_count(&store.storage).await, 3);
}

#[tokio::test]
async fn the_associations_command_returns_the_real_edges() {
    // §4 参照: every edge touching one Memory, in **both** directions.
    let store = TempStore::new("associations").await;
    let old = create(&store.storage, "API経由の旧ノード").await;
    let new = create(&store.storage, "API経由の新ノード").await;
    // The mirror edge is only reachable through the §3 transition: `supersede`
    // writes `superseded_by_id` and `sync_links` derives the edge from it.
    MemoryService::supersede(&store.storage, &old.id, Some(&new.id))
        .await
        .expect("supersede");

    // From the target: the edge whose `target_memory_id` is this row.
    let by_target = MemoryService::associations_for(&store.storage, &old.id, 20)
        .await
        .expect("associations");
    assert_eq!(by_target.len(), 1);
    assert_eq!(by_target[0].source_memory_id, new.id);
    assert_eq!(by_target[0].target_memory_id, old.id);
    assert_eq!(by_target[0].kind, "supersedes");
    assert!((by_target[0].weight - LINK_DEFAULT_WEIGHT).abs() < f64::EPSILON);
    assert_eq!(by_target[0].coactivation_count, 0);

    // From the source: the very same row, not a mirror of it.
    let by_source = MemoryService::associations_for(&store.storage, &new.id, 20)
        .await
        .expect("associations");
    assert_eq!(by_source.len(), 1);
    assert_eq!(by_source[0].id, by_target[0].id);

    // §4's endpoint checks the key first: an id that is not yours is a 404,
    // not an empty list.
    let err = MemoryService::associations_for(&store.storage, "no-such-memory", 20)
        .await
        .expect_err("an unknown id must fail");
    assert!(err.to_string().contains("memory not found"));

    // The default (20) and a caller's own limit both cut the list.
    assert_eq!(
        MemoryService::associations_for(&store.storage, &old.id, 0)
            .await
            .expect("associations")
            .len(),
        1,
        "limit 0 still returns the edges"
    );
}

#[tokio::test]
async fn extraction_proposes_links_to_the_rows_it_agrees_with() {
    // §4 source (1): §6's prompt is a byte-for-byte cross-edition contract, so
    // the proposal is derived *after* the rows land rather than asked for.
    let store = TempStore::new("suggest").await;
    let stored = create(&store.storage, "formal_language_source では要約を省く").await;

    let run_id = Uuid::new_v4().to_string();
    let conversation_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, '{}')")
        .bind(&conversation_id)
        .bind(SINGLE_USER_ID)
        .execute(store.storage.pool())
        .await
        .expect("insert conversation");
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, '{}', ?, 'queued', ?)",
    )
    .bind(&run_id)
    .bind(SINGLE_USER_ID)
    .bind(&conversation_id)
    .bind(format!("key-{run_id}"))
    .execute(store.storage.pool())
    .await
    .expect("insert run");

    let applied = apply_extraction(
        &store.storage,
        &[ExtractedMemory {
            content: "formal_language_source では箇条書きを三行に収める".to_string(),
            role: "preference".to_string(),
            action: "CREATE".to_string(),
            reason: String::new(),
        }],
        &run_id,
    )
    .await
    .expect("apply extraction");

    assert_eq!(applied.created.len(), 1);
    let created_id = applied.created[0].clone();
    assert_ne!(created_id, stored.id, "a new row, not a reinforcement");

    // One proposal, one edge, pointing at the row the extracted terms overlap.
    let links = all_links(&store.storage).await;
    assert!(!links.is_empty(), "the extraction must propose something");
    assert!(
        links.len() <= LINK_SUGGEST_LIMIT,
        "§4 caps at three per row"
    );
    for link in &links {
        assert_eq!(link.source_memory_id, created_id);
        assert_eq!(link.target_memory_id, stored.id);
        assert_eq!(link.kind, "relates_to");
    }

    // The upsert is idempotent: a second pass over the same rows is free.
    let again = suggest_links(&store.storage, &applied.created)
        .await
        .expect("suggest again");
    assert_eq!(again, 0, "nothing new was proposed");
    assert_eq!(link_count(&store.storage).await, links.len());
}

#[tokio::test]
async fn embedding_neighbours_stay_off_unless_the_setting_turns_them_on() {
    // §4 source (3): 「(3) は既定オフ」 — the gate refuses before the query runs.
    let store = TempStore::new("embedding").await;
    let first = create(&store.storage, "埋め込み近傍の基準ノード").await;
    let second = create(&store.storage, "埋め込み近傍の探索ノード").await;
    for id in [&first.id, &second.id] {
        MemoryService::write_embedding(
            &store.storage,
            id,
            "test-model",
            &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        )
        .await
        .expect("write embedding");
    }

    // No `app_settings` row at all: absent means off.
    assert_eq!(
        embedding_neighbor_links_if_enabled(&store.storage, std::slice::from_ref(&second.id))
            .await
            .expect("gate"),
        0,
        "the setting ships off"
    );
    assert_eq!(link_count(&store.storage).await, 0);

    // A setting that is not there yet is still off — the row exists and says
    // nothing about the flag.
    set_setting(&store.storage, json!({})).await;
    assert_eq!(
        embedding_neighbor_links_if_enabled(&store.storage, std::slice::from_ref(&second.id))
            .await
            .expect("gate"),
        0
    );
    assert_eq!(link_count(&store.storage).await, 0);

    // Turned on, the neighbour is found through §5's embedding: identical
    // vectors are a similarity of 1.0, well above the §4 floor.
    set_setting(
        &store.storage,
        json!({ "memory_link_embedding_neighbors": true }),
    )
    .await;
    let created =
        embedding_neighbor_links_if_enabled(&store.storage, std::slice::from_ref(&second.id))
            .await
            .expect("gate open");
    assert_eq!(created, 1);
    let links = all_links(&store.storage).await;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source_memory_id, second.id);
    assert_eq!(links[0].target_memory_id, first.id);
    assert_eq!(links[0].kind, "relates_to");
    assert!(
        links[0].weight >= LINK_NEIGHBOR_MIN_SIMILARITY,
        "the §4 floor is on the normalised cosine: {}",
        links[0].weight
    );

    // Idempotent like every other §4 writer: no second row, no second edge.
    assert_eq!(
        embedding_neighbor_links_if_enabled(&store.storage, std::slice::from_ref(&second.id))
            .await
            .expect("gate open again"),
        0
    );
    assert_eq!(link_count(&store.storage).await, 1);
}

/// Write `app_settings`'s singleton row with `data`.
async fn set_setting(storage: &Storage, data: Value) {
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(SINGLE_USER_ID)
    .bind(data.to_string())
    .execute(storage.pool())
    .await
    .expect("write app_settings");
}
