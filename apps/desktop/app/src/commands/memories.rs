use crate::AppState;
use mixd_core::memory::{MemoryCreate as CoreCreate, MemoryItem, MemoryService};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct MemoryView {
    pub id: String,
    pub title: String,
    pub body: String,
    pub role: String,
    pub scope: String,
    pub source_kind: String,
    pub lifecycle_state: String,
    pub verification: String,
    pub confidence: f64,
    pub salience: f64,
    pub strength: f64,
    /// §1 `pinned`: held, exempt from decay and always retrievable (§3).
    pub pinned: bool,
    /// §1 `superseded_by_id` — who replaced this row. `established` rows carry
    /// `null`; a `superseded` row names its replacement (§3).
    pub superseded_by_id: Option<String>,
    /// §1 `disputed_by_id` — who contradicts this row. Set on exactly the side
    /// §3's `dispute` marked, and cleared again by `resolve_conflict`.
    pub disputed_by_id: Option<String>,
    /// §4: how many `memory_links` edges touch this row, either direction.
    ///
    /// Omitted rather than sent as `0` — a Memory with no edges has no §4 story
    /// to tell, and a badge reading "0 links" is noise the panel should not
    /// have to filter out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_count: Option<u64>,
    pub updated_at: String,
}

impl From<MemoryItem> for MemoryView {
    fn from(m: MemoryItem) -> Self {
        Self {
            id: m.id,
            title: m.title,
            body: m.content,
            role: m.role,
            scope: m.scope,
            source_kind: m.source_kind,
            lifecycle_state: m.lifecycle_state,
            verification: m.verification,
            confidence: m.confidence,
            salience: m.salience,
            strength: m.strength,
            pinned: m.pinned,
            // Filled in by [`decorate`]: `MemoryItem` is §1's row as the tool
            // contract sees it, and the two §3 columns are not part of that.
            superseded_by_id: None,
            disputed_by_id: None,
            link_count: None,
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

/// Fill the §3 pointers and §4's `link_count` for a page of views.
///
/// Two batched queries for the whole page, never one per row: a Memory list of
/// 200 cards would otherwise issue 401 round trips to draw badges. The
/// `link_count` mirrors Docker's `service.link_counts` — one edge counted once
/// against **each** of its two endpoints, so a `supersedes` edge adds 1 to both
/// the replacement and the row it replaced.
///
/// `pub` (not `pub(crate)`) for the same reason `read_settings` is: it is the
/// part of the command with SQL worth exercising, and `#[tauri::command]`
/// functions themselves cannot be called from an integration test.
pub async fn decorate(
    storage: &mixd_storage::Storage,
    mut views: Vec<MemoryView>,
) -> anyhow::Result<Vec<MemoryView>> {
    if views.is_empty() {
        return Ok(views);
    }
    use sqlx::Row;
    let ids: Vec<String> = views.iter().map(|view| view.id.clone()).collect();
    let placeholders = vec!["?"; ids.len()].join(",");

    let pointer_sql = format!(
        "SELECT id, superseded_by_id, disputed_by_id FROM memories WHERE id IN ({placeholders})"
    );
    let mut pointers = sqlx::query(&pointer_sql);
    for id in &ids {
        pointers = pointers.bind(id);
    }
    let mut by_id = std::collections::HashMap::new();
    for row in pointers.fetch_all(storage.pool()).await? {
        let id: String = row.try_get("id")?;
        by_id.insert(
            id,
            (
                row.try_get::<Option<String>, _>("superseded_by_id")?,
                row.try_get::<Option<String>, _>("disputed_by_id")?,
            ),
        );
    }

    // Both directions at once: `OR` over two `IN`s, one pass over the edges
    // that touch this page. An edge between two rows on the page is fetched
    // once and counted twice below — once against each endpoint, which is what
    // "how many edges touch this row" means.
    let edge_sql = format!(
        "SELECT source_memory_id, target_memory_id FROM memory_links \
         WHERE owner_id = ? AND (source_memory_id IN ({placeholders}) \
         OR target_memory_id IN ({placeholders}))"
    );
    let mut edges = sqlx::query(&edge_sql).bind(mixd_storage::records::SINGLE_USER_ID);
    for id in &ids {
        edges = edges.bind(id);
    }
    for id in &ids {
        edges = edges.bind(id);
    }
    let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for row in edges.fetch_all(storage.pool()).await? {
        let source: String = row.try_get("source_memory_id")?;
        let target: String = row.try_get("target_memory_id")?;
        *counts.entry(source).or_default() += 1;
        *counts.entry(target).or_default() += 1;
    }

    for view in &mut views {
        if let Some((superseded_by_id, disputed_by_id)) = by_id.get(&view.id) {
            view.superseded_by_id = superseded_by_id.clone();
            view.disputed_by_id = disputed_by_id.clone();
        }
        view.link_count = counts.get(&view.id).copied().filter(|count| *count > 0);
    }
    Ok(views)
}

/// [`MemoryView`] for one row, pointers and edge count included.
async fn view_of(storage: &mixd_storage::Storage, item: MemoryItem) -> Result<MemoryView, String> {
    decorate(storage, vec![MemoryView::from(item)])
        .await
        .map_err(crate::error::cmd_err)
        .and_then(|mut views| {
            views
                .pop()
                .ok_or_else(|| "memory view missing after decorate".to_string())
        })
}

#[tauri::command]
pub async fn list_memories(state: State<'_, AppState>) -> Result<Vec<MemoryView>, String> {
    let items = MemoryService::list(state.storage.as_ref(), 200)
        .await
        .map_err(crate::error::cmd_err)?;
    decorate(
        state.storage.as_ref(),
        items.into_iter().map(MemoryView::from).collect(),
    )
    .await
    .map_err(crate::error::cmd_err)
}

#[derive(Deserialize)]
pub struct MemoryCreate {
    pub title: String,
    pub body: String,
    pub role: Option<String>,
    pub scope: Option<String>,
    pub source_kind: Option<String>,
}

#[tauri::command]
pub async fn create_memory(
    state: State<'_, AppState>,
    input: MemoryCreate,
) -> Result<MemoryView, String> {
    let item = MemoryService::create(
        state.storage.as_ref(),
        CoreCreate {
            title: input.title,
            content: input.body,
            role: input.role.unwrap_or_else(|| "fact".into()),
            scope: input.scope.unwrap_or_else(|| "user".into()),
            source_kind: input.source_kind.unwrap_or_else(|| "user".into()),
            task_id: None,
        },
    )
    .await
    .map_err(crate::error::cmd_err)?;
    // The row exists now; embedding it in the background keeps a provider
    // without an embeddings endpoint from delaying the save (spec §5.2).
    crate::spawn_memory_embed(
        state.storage.clone(),
        state.embeddings.clone(),
        item.id.clone(),
    );
    view_of(state.storage.as_ref(), item).await
}

#[derive(Deserialize)]
pub struct MemoryUpdate {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[tauri::command]
pub async fn update_memory(
    state: State<'_, AppState>,
    input: MemoryUpdate,
) -> Result<MemoryView, String> {
    let item = MemoryService::update(state.storage.as_ref(), &input.id, &input.title, &input.body)
        .await
        .map_err(crate::error::cmd_err)?;
    // `update` drops the row's embedding with the text it no longer
    // describes, so this re-embed is what puts a current one back; until it
    // lands the row ranks lexically rather than with a stale vector.
    crate::spawn_memory_embed(
        state.storage.clone(),
        state.embeddings.clone(),
        item.id.clone(),
    );
    view_of(state.storage.as_ref(), item).await
}

#[tauri::command]
pub async fn delete_memory(state: State<'_, AppState>, id: String) -> Result<(), String> {
    MemoryService::delete(state.storage.as_ref(), &id)
        .await
        .map_err(crate::error::cmd_err)
}

#[derive(Serialize)]
pub struct MemoryRevisionView {
    pub id: String,
    pub title: String,
    pub content: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_memory_revisions(
    state: State<'_, AppState>,
    memory_id: String,
) -> Result<Vec<MemoryRevisionView>, String> {
    let revisions = MemoryService::revisions(state.storage.as_ref(), &memory_id)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(revisions
        .into_iter()
        .map(|r| MemoryRevisionView {
            id: r.id,
            title: r.title,
            content: r.content,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub async fn restore_memory_revision(
    state: State<'_, AppState>,
    memory_id: String,
    revision_id: String,
) -> Result<MemoryView, String> {
    let item = MemoryService::restore(state.storage.as_ref(), &memory_id, &revision_id)
        .await
        .map_err(crate::error::cmd_err)?;
    // The embedding went with the text it described — `restore` rebuilds the
    // stored vector's row exactly as `update` does — so re-embed in the
    // background for the same reason.
    crate::spawn_memory_embed(
        state.storage.clone(),
        state.embeddings.clone(),
        item.id.clone(),
    );
    view_of(state.storage.as_ref(), item).await
}

#[tauri::command]
pub async fn verify_memory(
    state: State<'_, AppState>,
    id: String,
    verdict: String,
) -> Result<MemoryView, String> {
    MemoryService::verify(state.storage.as_ref(), &id, &verdict)
        .await
        .map_err(crate::error::cmd_err)?;
    let item = MemoryService::get(state.storage.as_ref(), &id)
        .await
        .map_err(crate::error::cmd_err)?;
    view_of(state.storage.as_ref(), item).await
}

#[derive(Serialize)]
pub struct MemorySearchHit {
    pub id: String,
    pub title: String,
    pub content: String,
    pub role: String,
    pub scope: String,
    pub score: f64,
    /// §4: `"リンクから"` on a row the 1-hop expansion pulled in on an edge,
    /// `null` on a row the query matched. Exposed here (rather than only in
    /// the debug block) so the UI can tell the two apart without a second
    /// call — Phase 5's screen is what renders it.
    pub selection_reason: Option<String>,
}

#[tauri::command]
pub async fn search_memories(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MemorySearchHit>, String> {
    // Hybrid rather than lexical: the resolver answers `None` when nothing
    // can embed, and `search_hybrid` then ranks exactly like `search` did
    // before Phase 1 — the vector term is strictly additive (spec §5.2).
    let source = state.embeddings.resolve().await;
    let hits = MemoryService::search_hybrid(
        state.storage.as_ref(),
        &query,
        limit
            .unwrap_or(20)
            .min(mixd_core::retrieval::RESULT_LIMIT_MAX),
        source.as_ref(),
    )
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(hits
        .into_iter()
        .map(|h| MemorySearchHit {
            id: h.id,
            title: h.title,
            content: h.content,
            role: h.role,
            scope: h.scope,
            score: h.score,
            selection_reason: h.selection_reason,
        })
        .collect())
}

/// §4's reference API: every `memory_links` edge touching one Memory, in
/// **both** directions, best weight first.
///
/// The Docker edition serves this from `GET /memories/{key}/associations`
/// (spec §4 「参照: …」); this is its Tauri equivalent — there is no HTTP
/// surface here, and Phase 5 is the screen that calls it.
///
/// The name deliberately echoes the endpoint, *not* the legacy table: the
/// table §4 defines is `memory_links`, and §9 #8's entry about a
/// `memory_associations` table whose shape differed from §4 is resolved by
/// that table's removal (see `storage/src/migrations/run.rs`).
#[tauri::command]
pub async fn memory_associations(
    state: State<'_, AppState>,
    memory_id: String,
    limit: Option<usize>,
) -> Result<Vec<MemoryLinkView>, String> {
    let links = MemoryService::associations_for(
        state.storage.as_ref(),
        &memory_id,
        // §4's endpoint default is 20; a caller asking for fewer gets fewer
        // and one asking for more is capped by §2's own `result_limit_max`
        // rather than by an arbitrary ceiling of ours.
        limit
            .unwrap_or(20)
            .min(mixd_core::retrieval::RESULT_LIMIT_MAX),
    )
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(links.into_iter().map(MemoryLinkView::from).collect())
}

/// One `memory_links` row, as the UI reads it (spec §4).
#[derive(Serialize)]
pub struct MemoryLinkView {
    pub id: String,
    pub source_memory_id: String,
    pub target_memory_id: String,
    pub kind: String,
    pub weight: f64,
    pub coactivation_count: i64,
    pub updated_at: String,
}

impl From<mixd_core::memory_links::MemoryLink> for MemoryLinkView {
    fn from(link: mixd_core::memory_links::MemoryLink) -> Self {
        Self {
            id: link.id,
            source_memory_id: link.source_memory_id,
            target_memory_id: link.target_memory_id,
            kind: link.kind,
            weight: link.weight,
            coactivation_count: link.coactivation_count,
            updated_at: link.updated_at.to_rfc3339(),
        }
    }
}

/// What a settled dispute left behind (spec §3 `resolve_conflict`).
#[derive(Serialize)]
pub struct ConflictResolutionView {
    /// The adopted Memory, back to `active`.
    pub winner: String,
    /// The rejected Memory; §3 deletes neither side.
    pub loser: String,
    /// `superseded`, or `expired` when the caller resolved as a dismissal.
    pub loser_state: String,
    /// The resolution string the caller passed through, echoed back.
    pub resolution: String,
}

/// Spec §3: settle a dispute without losing either side's history.
///
/// Three things this deliberately does *not* do:
///
/// * it does not pick a winner for the caller. With no `prefer`, §3's own rule
///   stands — **the already-stored side wins**, i.e. the side the disputed row's
///   `disputed_by_id` points at. The UI may only override that when the user
///   names a side, which is what `prefer` is for.
/// * it does not skip §3's revisions: `resolve_conflict` writes one for each
///   side inside its own transaction before either column moves.
/// * it does not delete. The rejected row keeps its content and its history and
///   leaves recall, exactly as §3 says.
///
/// `resolution` decides the rejected row's fate: `dismiss` / `dismissed` /
/// `rejected` / `discard` make it `expired`, anything else `superseded`.
#[tauri::command]
pub async fn resolve_conflict(
    state: State<'_, AppState>,
    conflict_id: String,
    resolution: String,
    prefer: Option<String>,
) -> Result<ConflictResolutionView, String> {
    let resolved = MemoryService::resolve_conflict(
        state.storage.as_ref(),
        &conflict_id,
        &resolution,
        prefer.as_deref().filter(|id| !id.is_empty()),
    )
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(ConflictResolutionView {
        winner: resolved.winner,
        loser: resolved.loser,
        loser_state: resolved.loser_state,
        resolution: resolved.resolution,
    })
}
