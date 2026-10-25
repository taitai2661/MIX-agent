//! Integration coverage for the Memory command layer's page decorator.
//!
//! [`decorate`] is the only new SQL in the Phase 5 command surface: it reads
//! §1's two §3 pointer columns and counts §4's edges so the pane can draw
//! badges. `#[tauri::command]` functions cannot be called from a test, so the
//! helper they all route through is what gets exercised here — the same shape
//! `app/tests/settings.rs` uses for `read_settings`.
//!
//! Rows are fetched fresh before decorating, the way `list_memories` and
//! `view_of` do: a `MemoryItem` handed back by `create` carries the lifecycle
//! it was created with, and a test that decorated *that* would be asserting on
//! a value the command layer never sends.

use mix_agent_desktop::commands::memories::{decorate, MemoryView};
use mixd_core::memory::{MemoryCreate, MemoryItem, MemoryService};
use mixd_storage::Storage;

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-memories-test-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

async fn create(storage: &Storage, title: &str, content: &str) -> MemoryItem {
    MemoryService::create(
        storage,
        MemoryCreate {
            title: title.into(),
            content: content.into(),
            role: "fact".into(),
            scope: "user".into(),
            source_kind: "user".into(),
            task_id: None,
        },
    )
    .await
    .expect("create memory")
}

/// Re-read the row and decorate it, exactly as the commands do.
async fn view(storage: &Storage, id: &str) -> MemoryView {
    let item = MemoryService::get(storage, id).await.expect("get memory");
    let mut views = decorate(storage, vec![MemoryView::from(item)])
        .await
        .expect("decorate");
    views.pop().expect("one view in, one view out")
}

#[tokio::test]
async fn a_row_with_no_edges_reports_no_link_count() {
    let storage = temp_storage().await;
    let item = create(&storage, "孤立した記憶", "Nobody links here yet.").await;

    let row = view(&storage, &item.id).await;
    // §4's badge is omitted rather than sent as `0`: "no edges" and "not
    // loaded" have to stay distinguishable, and `None` is the former.
    assert_eq!(row.link_count, None, "an unlinked row must not claim links");
    assert_eq!(row.superseded_by_id, None);
    assert_eq!(row.disputed_by_id, None);
    // §1's own columns ride along untouched by the decorator.
    assert_eq!(row.lifecycle_state, "established");
    assert_eq!(row.source_kind, "user");
    assert!((row.confidence - 0.7).abs() < f64::EPSILON);
    assert!(!row.pinned, "a fresh row is not pinned");
}

#[tokio::test]
async fn one_edge_is_counted_against_both_of_its_endpoints() {
    let storage = temp_storage().await;
    let first = create(&storage, "始点の記憶", "The edge starts here.").await;
    let second = create(&storage, "終点の記憶", "The edge ends here.").await;

    MemoryService::link(&storage, &second.id, &first.id, "relates_to")
        .await
        .expect("link");

    let a = view(&storage, &first.id).await;
    let b = view(&storage, &second.id).await;
    // Docker's `service.link_counts` does the same: how many edges *touch* a
    // row, so both ends see the edge.
    assert_eq!(a.link_count, Some(1));
    assert_eq!(b.link_count, Some(1));
    assert_eq!(a.superseded_by_id, None);
    assert_eq!(b.superseded_by_id, None);
}

/// The §4 → §3 direction is closed at `link()`, so the only way a
/// `superseded_by_id` can exist is a §3 transition — which means the pointer
/// the pane renders and the edge it counts have to arrive together.
#[tokio::test]
async fn a_supersede_pointer_and_its_mirror_edge_arrive_together() {
    let storage = temp_storage().await;
    let old = create(&storage, "旧しい方", "The fact that got replaced.").await;
    let new = create(&storage, "新しい方", "The replacement.").await;

    MemoryService::supersede(&storage, &old.id, Some(&new.id))
        .await
        .expect("supersede");

    let old_row = view(&storage, &old.id).await;
    let new_row = view(&storage, &new.id).await;

    assert_eq!(old_row.superseded_by_id.as_deref(), Some(new.id.as_str()));
    assert_eq!(old_row.lifecycle_state, "superseded");
    assert_eq!(
        old_row.link_count,
        Some(1),
        "the mirror edge touches the old row"
    );

    assert_eq!(
        new_row.superseded_by_id, None,
        "the replacement carries no §3 pointer"
    );
    assert_eq!(
        new_row.link_count,
        Some(1),
        "the same edge touches the replacement too"
    );
}

#[tokio::test]
async fn a_dispute_names_the_other_side_and_counts_the_contradicts_edge() {
    let storage = temp_storage().await;
    let disputed = create(&storage, "疑われた方", "The claim under dispute.").await;
    let challenger = create(&storage, "疑う方", "The contradicting claim.").await;

    MemoryService::dispute(&storage, &disputed.id, &challenger.id)
        .await
        .expect("dispute");

    let marked = view(&storage, &disputed.id).await;
    let other = view(&storage, &challenger.id).await;

    assert_eq!(
        marked.disputed_by_id.as_deref(),
        Some(challenger.id.as_str())
    );
    assert_eq!(marked.lifecycle_state, "disputed");
    assert_eq!(marked.link_count, Some(1));
    // §3 marks **one row only**: the contradicting side stays retrievable and
    // carries no `disputed_by_id` of its own.
    assert_eq!(other.disputed_by_id, None);
    assert_eq!(other.lifecycle_state, "established");
    assert_eq!(other.link_count, Some(1));
}
