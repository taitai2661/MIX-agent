//! Spec §3 lifecycle: `supersede` / `dispute` / `resolve_conflict` / `decay`,
//! the retrieval gate over `retrievable_lifecycles`, and the §3 initial
//! lifecycle of an explicit vs an automatically formed Memory.
//!
//! Everything runs against a temporary SQLite file, so the suite never touches
//! the network. The contract of record is `desktop/docs/memory-spec.md` §1–§3.

use chrono::{Duration as ChronoDuration, Utc};
use mixd_core::memory::{
    apply_extraction, handle_explicit, ExplicitOutcome, ExtractedMemory, MemoryCreate, MemoryItem,
    MemoryService, AUTO_INITIAL_LIFECYCLE, EXPLICIT_INITIAL_LIFECYCLE, LIFECYCLE_STATES,
    RETRIEVABLE_LIFECYCLES,
};
use mixd_core::retrieval::{retrieve_all, retrieve_memories, retrieve_memories_hybrid};
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::Storage;
use serde_json::json;
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
            "mixd-memory-lifecycle-{tag}-{}-{:?}",
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

/// A Memory whose title and content both carry `marker`, so one query word
/// finds it and nothing else in the store does.
async fn create_marked(storage: &Storage, marker: &str) -> MemoryItem {
    MemoryService::create(
        storage,
        MemoryCreate {
            title: format!("note about {marker}"),
            content: format!("the {marker} record is kept up to date"),
            role: "fact".to_string(),
            scope: "user".to_string(),
            source_kind: "user".to_string(),
            task_id: None,
        },
    )
    .await
    .expect("create memory")
}

/// The §1 columns a lifecycle transition or the recall gate can touch.
async fn columns(storage: &Storage, id: &str) -> (String, String, f64, f64, f64, i64) {
    let row = sqlx::query(
        "SELECT lifecycle_state, updated_at, salience, strength, confidence, \
         activation_count FROM memories WHERE id = ?",
    )
    .bind(id)
    .fetch_one(storage.pool())
    .await
    .expect("memory row");
    (
        row.get("lifecycle_state"),
        row.get("updated_at"),
        row.get("salience"),
        row.get("strength"),
        row.get("confidence"),
        row.get("activation_count"),
    )
}

/// Rewind a Memory's recency — decay's input — without touching anything else.
async fn set_updated_at(storage: &Storage, id: &str, ts: chrono::DateTime<Utc>) {
    sqlx::query("UPDATE memories SET updated_at = ? WHERE id = ?")
        .bind(ts.to_rfc3339())
        .bind(id)
        .execute(storage.pool())
        .await
        .expect("set updated_at");
}

async fn set_lifecycle(storage: &Storage, id: &str, state: &str) {
    sqlx::query("UPDATE memories SET lifecycle_state = ? WHERE id = ?")
        .bind(state)
        .bind(id)
        .execute(storage.pool())
        .await
        .expect("set lifecycle_state");
}

async fn revision_count(storage: &Storage, id: &str) -> usize {
    MemoryService::revisions(storage, id)
        .await
        .expect("read revisions")
        .len()
}

/// Ids of the rows `query` recalls, in rank order.
async fn hits(storage: &Storage, query: &str) -> Vec<String> {
    retrieve_memories(storage, query, 10)
        .await
        .expect("retrieve_memories")
        .into_iter()
        .map(|hit| hit.id)
        .collect()
}

/// The run row `apply_extraction`'s `memory_action_events.run_id` foreign key
/// needs; the extraction itself never reads the run's payload.
async fn seed_run(storage: &Storage) -> String {
    let conversation_id = Uuid::new_v4();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(conversation_id.to_string())
        .bind(SINGLE_USER_ID)
        .bind("{}")
        .execute(storage.pool())
        .await
        .expect("insert conversation");
    let run_id = format!("run-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, ?, ?, 'queued', ?)",
    )
    .bind(&run_id)
    .bind(SINGLE_USER_ID)
    .bind(json!({"modelId": "p1:m", "providerId": "p1"}).to_string())
    .bind(conversation_id.to_string())
    .bind(format!("key-{run_id}"))
    .execute(storage.pool())
    .await
    .expect("insert run");
    run_id
}

// ---------------------------------------------------------------------------
// §3 supersede — a soft transition, never a delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn supersede_retires_the_old_row_without_deleting_it() {
    let store = TempStore::new("supersede").await;
    let old = create_marked(&store.storage, "quoxin").await;
    let new = create_marked(&store.storage, "quoxin").await;

    MemoryService::supersede(&store.storage, &old.id, Some(&new.id))
        .await
        .expect("supersede");

    // §3 「行を消さない」: the row is still there, with its content intact.
    let survived = MemoryService::get(&store.storage, &old.id)
        .await
        .expect("old row");
    assert_eq!(survived.content, old.content);
    assert_eq!(survived.lifecycle_state, "superseded");
    // §1: `superseded_by_id` names the replacement.
    let superseded_by: String =
        sqlx::query_scalar("SELECT superseded_by_id FROM memories WHERE id = ?")
            .bind(&old.id)
            .fetch_one(store.storage.pool())
            .await
            .expect("superseded_by_id");
    assert_eq!(superseded_by, new.id);
    // §1 「改変のたびに 1 行 memory_revisions へ」: create + transition.
    assert_eq!(revision_count(&store.storage, &old.id).await, 2);

    // §2: the retired row left recall; the replacement is what serves it.
    let found = hits(&store.storage, "quoxin").await;
    assert!(
        !found.contains(&old.id),
        "superseded row must not be recalled: {found:?}"
    );
    assert!(
        found.contains(&new.id),
        "replacement must be recalled: {found:?}"
    );
}

#[tokio::test]
async fn supersede_rejects_a_missing_or_self_referential_replacement() {
    let store = TempStore::new("supersede-err").await;
    let old = create_marked(&store.storage, "vellum").await;
    let other = create_marked(&store.storage, "vellum").await;

    // §3: a supersede without the replacement is refused, not degraded into a
    // hard delete.
    let no_new = MemoryService::supersede(&store.storage, &old.id, None).await;
    assert!(no_new.is_err(), "new_id=None must be refused");
    let blank = MemoryService::supersede(&store.storage, &old.id, Some("  ")).await;
    assert!(blank.is_err(), "a blank new_id must be refused");
    let myself = MemoryService::supersede(&store.storage, &old.id, Some(&old.id)).await;
    assert!(myself.is_err(), "a memory cannot supersede itself");
    let missing_old =
        MemoryService::supersede(&store.storage, "no-such-memory", Some(&other.id)).await;
    assert!(missing_old.is_err(), "a missing old row must be refused");
    let missing_new =
        MemoryService::supersede(&store.storage, &old.id, Some("no-such-memory")).await;
    assert!(
        missing_new.is_err(),
        "a missing replacement must be refused"
    );

    // Every refusal left the row untouched and recallable.
    let (state, _, _, _, _, _) = columns(&store.storage, &old.id).await;
    assert_eq!(state, "established");
    assert!(hits(&store.storage, "vellum").await.contains(&old.id));
}

// ---------------------------------------------------------------------------
// §1 revisions — history survives every transition, and can be restored
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revisions_are_left_behind_and_restorable() {
    let store = TempStore::new("revisions").await;
    let item = create_marked(&store.storage, "lumen").await;
    assert_eq!(revision_count(&store.storage, &item.id).await, 1);

    MemoryService::update(
        &store.storage,
        &item.id,
        "lumen note",
        "the lumen record changed",
    )
    .await
    .expect("update");
    assert_eq!(revision_count(&store.storage, &item.id).await, 2);

    // §1: the older revision still carries the original text.
    let mut revisions = MemoryService::revisions(&store.storage, &item.id)
        .await
        .expect("revisions");
    revisions.sort_by_key(|revision| revision.created_at);
    assert_eq!(revisions[0].content, item.content);

    let restored = MemoryService::restore(&store.storage, &item.id, &revisions[0].id)
        .await
        .expect("restore");
    assert_eq!(restored.content, item.content);
    // A restore is an edit like any other: it, too, leaves a revision (§1).
    assert_eq!(revision_count(&store.storage, &item.id).await, 3);
}

// ---------------------------------------------------------------------------
// §3 dispute / resolve_conflict
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_dispute_marks_only_one_side_and_resolve_defaults_to_the_stored_one() {
    let store = TempStore::new("dispute").await;
    let stored = create_marked(&store.storage, "vellum").await;
    let challenger = create_marked(&store.storage, "vellum").await;

    MemoryService::dispute(&store.storage, &challenger.id, &stored.id)
        .await
        .expect("dispute");

    // §3: only the named row is marked — the stored fact keeps serving recall.
    let (state, _, _, _, _, _) = columns(&store.storage, &challenger.id).await;
    assert_eq!(state, "disputed");
    let (state, _, _, _, _, _) = columns(&store.storage, &stored.id).await;
    assert_eq!(state, "established");
    let found = hits(&store.storage, "vellum").await;
    assert!(
        found.contains(&stored.id),
        "the stored side stays recallable: {found:?}"
    );
    assert!(
        !found.contains(&challenger.id),
        "the disputed side leaves recall: {found:?}"
    );

    let settled =
        MemoryService::resolve_conflict(&store.storage, &challenger.id, "confirmed", None)
            .await
            .expect("resolve_conflict");
    // §3: with no preference the challenger does not win — the already-stored
    // side the dispute points at is adopted.
    assert_eq!(settled.winner, stored.id);
    assert_eq!(settled.loser, challenger.id);
    assert_eq!(settled.loser_state, "superseded");

    let (state, _, _, _, _, _) = columns(&store.storage, &stored.id).await;
    assert_eq!(state, "active");
    let (state, _, _, _, _, _) = columns(&store.storage, &challenger.id).await;
    assert_eq!(state, "superseded");
    let superseded_by: String =
        sqlx::query_scalar("SELECT superseded_by_id FROM memories WHERE id = ?")
            .bind(&challenger.id)
            .fetch_one(store.storage.pool())
            .await
            .expect("superseded_by_id");
    assert_eq!(superseded_by, stored.id);
    // Neither side is deleted, and §3 「両側に revisionを残す」.
    assert_eq!(revision_count(&store.storage, &challenger.id).await, 3); // create + dispute + resolve
    assert_eq!(revision_count(&store.storage, &stored.id).await, 2); // create + resolve
}

#[tokio::test]
async fn a_named_preference_decides_and_a_dismissal_expires_the_loser() {
    let store = TempStore::new("resolve").await;
    let stored = create_marked(&store.storage, "cobalt").await;
    let challenger = create_marked(&store.storage, "cobalt").await;

    MemoryService::dispute(&store.storage, &challenger.id, &stored.id)
        .await
        .expect("dispute");
    // A `prefer` outside the pair is not a valid settlement.
    let foreign = MemoryService::resolve_conflict(
        &store.storage,
        &challenger.id,
        "replaced",
        Some("no-such-memory"),
    )
    .await;
    assert!(
        foreign.is_err(),
        "prefer must be one of the two conflicting rows"
    );

    let settled = MemoryService::resolve_conflict(
        &store.storage,
        &challenger.id,
        "replaced",
        Some(&challenger.id),
    )
    .await
    .expect("resolve with prefer");
    assert_eq!(settled.winner, challenger.id);
    assert_eq!(settled.loser, stored.id);
    assert_eq!(settled.loser_state, "superseded");

    // §3: settling as a dismissal does not supersede — it expires the loser.
    let second_stored = create_marked(&store.storage, "cobalt").await;
    let second_challenger = create_marked(&store.storage, "cobalt").await;
    MemoryService::dispute(&store.storage, &second_challenger.id, &second_stored.id)
        .await
        .expect("second dispute");
    let dismissed =
        MemoryService::resolve_conflict(&store.storage, &second_challenger.id, "dismissed", None)
            .await
            .expect("resolve with dismissal");
    assert_eq!(dismissed.loser_state, "expired");
    let (state, _, _, _, _, _) = columns(&store.storage, &dismissed.loser).await;
    assert_eq!(state, "expired");

    // Settling a row that is not disputed is refused.
    let undisp =
        MemoryService::resolve_conflict(&store.storage, &stored.id, "confirmed", None).await;
    assert!(undisp.is_err(), "an undisputed row cannot be settled");
}

// ---------------------------------------------------------------------------
// §3 decay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn decay_weakens_idle_rows_on_a_180_day_half_life_and_archives_at_the_floor() {
    let store = TempStore::new("decay").await;
    let now = Utc::now();

    // A held Memory is exempt from decay outright (§3).
    let pinned = create_marked(&store.storage, "amber").await;
    MemoryService::set_pinned(&store.storage, &pinned.id, true)
        .await
        .expect("pin");
    set_updated_at(&store.storage, &pinned.id, now - ChronoDuration::days(5000)).await;

    // Exactly one half-life idle: every scalar halves.
    let halflife = create_marked(&store.storage, "basalt").await;
    set_updated_at(
        &store.storage,
        &halflife.id,
        now - ChronoDuration::days(180),
    )
    .await;

    // A daily pass has nothing to say about a Memory from this morning —
    // 0.4 days idle is under the 0.5-day floor.
    let fresh = create_marked(&store.storage, "cyanide").await;
    set_updated_at(
        &store.storage,
        &fresh.id,
        now - ChronoDuration::minutes(576),
    )
    .await;

    // Far past the floor: both scalars would land under 0.1.
    let doomed = create_marked(&store.storage, "dolomite").await;
    let doomed_updated_at = (now - ChronoDuration::days(5000)).to_rfc3339();
    sqlx::query("UPDATE memories SET updated_at = ? WHERE id = ?")
        .bind(&doomed_updated_at)
        .bind(&doomed.id)
        .execute(store.storage.pool())
        .await
        .expect("rewind doomed");

    let touched = MemoryService::decay(&store.storage).await.expect("decay");
    assert_eq!(touched, 2, "only the idle, unpinned rows decay");

    let (state, updated_at, salience, strength, _, _) = columns(&store.storage, &pinned.id).await;
    assert_eq!(state, "established", "a pinned row is never archived");
    assert_eq!(salience, 0.5, "a pinned row is never weakened");
    assert_eq!(strength, 0.6, "a pinned row is never weakened");
    // §3: pinning holds a Memory without changing its recency, so its
    // untouched `updated_at` is still the one the test rewound.
    assert_eq!(updated_at, (now - ChronoDuration::days(5000)).to_rfc3339());

    let (_, _, salience, strength, _, _) = columns(&store.storage, &halflife.id).await;
    // The row is rewound to 180 days and `decay` reads the clock a few
    // milliseconds later, so the exponent is 180.000…x days: the products are
    // 0.25 / 0.3 minus ~1e-8, not exactly on the nose.
    assert!(
        (salience - 0.25).abs() < 1e-6,
        "salience 0.5 · 0.5^(180/180) = 0.25, got {salience}"
    );
    assert!(
        (strength - 0.3).abs() < 1e-6,
        "strength 0.6 · 0.5^(180/180) = 0.3, got {strength}"
    );

    let (state, _, salience, strength, _, _) = columns(&store.storage, &fresh.id).await;
    assert_eq!(state, "established");
    assert_eq!(salience, 0.5, "a row idle for 0.4 days is left alone");
    assert_eq!(strength, 0.6, "a row idle for 0.4 days is left alone");

    let (state, updated_at, salience, strength, _, _) = columns(&store.storage, &doomed.id).await;
    assert_eq!(state, "archived", "a value at the floor archives the row");
    assert!(
        (salience - 0.1).abs() < 1e-9,
        "clamped at the floor, got {salience}"
    );
    assert!(
        (strength - 0.1).abs() < 1e-9,
        "clamped at the floor, got {strength}"
    );
    // §3: `updated_at` is the *input* to decay, so a decay never writes it.
    assert_eq!(updated_at, doomed_updated_at);

    // Scalar decay is a recomputation, not an edit: no revision for the row
    // that only decayed, and one DECAY event for the row that was archived.
    assert_eq!(revision_count(&store.storage, &halflife.id).await, 1);
    let decay_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memory_action_events WHERE data LIKE '%\"action\":\"DECAY\"%'",
    )
    .fetch_one(store.storage.pool())
    .await
    .expect("count DECAY events");
    assert_eq!(decay_events, 1);
    // And the archived row has left recall while the decayed one has not.
    assert!(!hits(&store.storage, "dolomite").await.contains(&doomed.id));
    assert!(hits(&store.storage, "basalt").await.contains(&halflife.id));
}

// ---------------------------------------------------------------------------
// §2/§3 the retrieval gate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retired_lifecycles_leave_recall_while_pinned_rows_stay() {
    let store = TempStore::new("gate").await;
    let archived = create_marked(&store.storage, "ember").await;
    set_lifecycle(&store.storage, &archived.id, "archived").await;
    assert!(
        !hits(&store.storage, "ember").await.contains(&archived.id),
        "an archived row is not retrievable"
    );

    // §3: `pinned` wins over any lifecycle — a held row is always retrievable.
    MemoryService::set_pinned(&store.storage, &archived.id, true)
        .await
        .expect("pin");
    assert!(
        hits(&store.storage, "ember").await.contains(&archived.id),
        "a pinned row is retrievable whatever its lifecycle says"
    );

    // §3: the four retired states are all out of recall, none deleted.
    for (marker, state) in [
        ("feldspar", "superseded"),
        ("gypsum", "expired"),
        ("hematite", "archived"),
        ("iodite", "disputed"),
    ] {
        let item = create_marked(&store.storage, marker).await;
        set_lifecycle(&store.storage, &item.id, state).await;
        assert!(
            !hits(&store.storage, marker).await.contains(&item.id),
            "{state} must not be retrievable"
        );
        assert_eq!(
            MemoryService::get(&store.storage, &item.id)
                .await
                .expect("row survives")
                .lifecycle_state,
            state,
            "{state} keeps its row"
        );
    }
    // The retrievable states still are.
    for (marker, state) in [
        ("jasper", "candidate"),
        ("kyanite", "active"),
        ("labrador", "established"),
    ] {
        let item = create_marked(&store.storage, marker).await;
        set_lifecycle(&store.storage, &item.id, state).await;
        assert!(
            hits(&store.storage, marker).await.contains(&item.id),
            "{state} must be retrievable"
        );
    }
}

// ---------------------------------------------------------------------------
// §3 initial lifecycle — explicit input vs automatic formation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn explicit_input_starts_established_and_auto_formation_starts_candidate() {
    let store = TempStore::new("initial").await;

    // §6's synchronous「覚えて：」intake is explicit human input (§3).
    let ExplicitOutcome { created, .. } =
        handle_explicit(&store.storage, "覚えて：zephyr station is on the roof")
            .await
            .expect("handle_explicit");
    let created = created.expect("the explicit save created a Memory");
    let (state, _, _, _, _, _) = columns(&store.storage, &created).await;
    assert_eq!(state, EXPLICIT_INITIAL_LIFECYCLE);
    assert_eq!(state, "established");

    // §6 post-response extraction is automatic formation (§3): `candidate`
    // until a transition confirms it — but still retrievable (§2).
    let run_id = seed_run(&store.storage).await;
    let applied = apply_extraction(
        &store.storage,
        &[ExtractedMemory {
            content: "the quartz registry refreshes nightly".to_string(),
            role: "fact".to_string(),
            action: "CREATE".to_string(),
            reason: "durable".to_string(),
        }],
        &run_id,
    )
    .await
    .expect("apply_extraction");
    let created = applied
        .created
        .first()
        .expect("the extractor created a Memory");
    let (state, _, _, _, _, _) = columns(&store.storage, created).await;
    assert_eq!(state, AUTO_INITIAL_LIFECYCLE);
    assert_eq!(state, "candidate");
    assert!(
        hits(&store.storage, "quartz").await.contains(created),
        "a candidate is still §2 retrievable"
    );
}

#[tokio::test]
async fn the_lifecycle_vocabulary_is_the_spec_ones() {
    // The parity script compares these against §2; the test pins the values so
    // a silent drift cannot pass review.
    assert_eq!(
        LIFECYCLE_STATES.to_vec(),
        vec![
            "candidate",
            "active",
            "established",
            "superseded",
            "expired",
            "archived",
            "disputed",
        ]
    );
    assert_eq!(
        RETRIEVABLE_LIFECYCLES.to_vec(),
        vec!["candidate", "active", "established"]
    );
    assert_eq!(EXPLICIT_INITIAL_LIFECYCLE, "established");
    assert_eq!(AUTO_INITIAL_LIFECYCLE, "candidate");
}

// ---------------------------------------------------------------------------
// §3 検索側の再激活経路 — recall reinforces, context injection does not
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_explicit_recall_reinforces_but_context_injection_does_not() {
    let store = TempStore::new("activate").await;
    let item = create_marked(&store.storage, "mica").await;

    // Context injection (§3): being injected every turn is not a re-activation.
    let block = retrieve_all(&store.storage, "mica")
        .await
        .expect("retrieve_all");
    assert!(block.memories.iter().any(|hit| hit.id == item.id));
    let (_, _, salience, strength, confidence, activations) =
        columns(&store.storage, &item.id).await;
    assert_eq!(activations, 0, "context injection must not reinforce");
    assert_eq!(salience, 0.5);
    assert_eq!(strength, 0.6);
    assert!((confidence - 0.7).abs() < 1e-9);

    // Explicit recall (§3): a recall is exactly when an activation counts.
    retrieve_memories(&store.storage, "mica", 5)
        .await
        .expect("retrieve_memories");
    let (_, _, salience, strength, confidence, activations) =
        columns(&store.storage, &item.id).await;
    assert_eq!(activations, 1);
    assert!(
        (salience - 0.55).abs() < 1e-9,
        "salience +0.05, got {salience}"
    );
    assert!(
        (strength - 0.63).abs() < 1e-9,
        "strength +0.03, got {strength}"
    );
    assert!(
        (confidence - 0.72).abs() < 1e-9,
        "confidence +0.02, got {confidence}"
    );

    // The hybrid entry is an explicit recall too — even when no source can
    // embed the query and it falls back to lexical ranking.
    retrieve_memories_hybrid(&store.storage, "mica", 5, None)
        .await
        .expect("retrieve_memories_hybrid");
    let (_, _, _, _, _, activations) = columns(&store.storage, &item.id).await;
    assert_eq!(activations, 2, "the hybrid recall path activates as well");
}
