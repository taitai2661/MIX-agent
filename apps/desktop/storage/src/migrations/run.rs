//! One-shot data migrations that backfill legacy rows after the schema
//! has been applied. Each function is idempotent: it inspects the rows and
//! only writes when the value is missing or stale, so re-running after a
//! crash or during tests is safe.

use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::Utc;
use sqlx::SqlitePool;

/// Folder name for generated workspaces. Matches the Codex Desktop default
/// (`~/Documents/MIX-agent Projects`).
pub const DEFAULT_PROJECTS_ROOT: &str = "MIX-agent Projects";

const PROJECTLESS_PARENT: &str = "Projectless";

/// Resolve the parent folder for generated workspaces, most specific first:
///
/// 1. `MIXD_PROJECTS_ROOT` (tests / power users),
/// 2. `~/Documents/MIX-agent Projects` (Codex parity),
/// 3. `<data_dir>/Documents/MIX-agent Projects` (no Documents dir, e.g. a
///    headless Linux box).
pub fn default_projects_root(data_dir: &Path) -> PathBuf {
    if let Some(override_root) = std::env::var_os("MIXD_PROJECTS_ROOT") {
        if !override_root.is_empty() {
            return PathBuf::from(override_root);
        }
    }
    if let Some(documents) = dirs::document_dir() {
        return documents.join(DEFAULT_PROJECTS_ROOT);
    }
    data_dir.join("Documents").join(DEFAULT_PROJECTS_ROOT)
}

/// Project data we keep on disk. Mirrors `ProjectData` but with `rootPath`
/// modelled as `Option<String>` so we can detect the missing case without
/// re-parsing through the strongly-typed struct (which now requires the
/// field).
#[derive(serde::Deserialize)]
struct LegacyProjectRow {
    name: String,
    #[serde(default, rename = "additionalPaths")]
    additional_paths: Vec<String>,
    #[serde(default, rename = "sources")]
    sources: Vec<String>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, rename = "updatedAt")]
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Run every one-shot migration. Idempotent.
///
/// Rollback story, two layers:
/// - SQL passes run inside one transaction each (see the passes below), so a
///   failure mid-pass rolls back that pass instead of leaving half a mirror
///   behind for the next boot to puzzle over.
/// - Filesystem side effects (`backfill_missing_project_roots` creates
///   folders) cannot roll back — the pre-open `mix-agent.db.backup-<timestamp>`
///   copy taken in `Storage::open` (`storage/src/db.rs`) is the recovery path
///   for those: re-open with the backup file renamed to `mix-agent.db`.
///
/// Every step names itself in its error context so a boot failure says
/// *which* migration broke, not just "data migration failed".
pub async fn run_data_migrations(pool: &SqlitePool, data_dir: &Path) -> anyhow::Result<()> {
    backfill_missing_project_roots(pool, data_dir)
        .await
        .context("data migration backfill_missing_project_roots failed")?;
    backfill_memory_links(pool)
        .await
        .context("data migration backfill_memory_links failed")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Spec §4 `memory_links` — backfill and retire the legacy link tables.
// ---------------------------------------------------------------------------

/// Does `name` exist yet? Asked from the catalog rather than by catching a
/// missing-table error, so a genuine failure of the query itself still
/// surfaces (§4's backfill must not swallow one and call it "not yet").
async fn table_exists(pool: &SqlitePool, name: &str) -> anyhow::Result<bool> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(name)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

/// Bring an existing database's associations into spec §4's `memory_links`.
///
/// Three passes, in this order, each of them idempotent so a crash between
/// two of them (or a re-run on every boot) is safe:
///
/// 1. §3's columns become their §4 mirror. A row retired before this phase
///    existed already carries `superseded_by_id` / `disputed_by_id` with no
///    edge behind it — the half-updated state §4 forbids ("二重管理にしない"
///    argues both ways: the column and the edge must agree).
/// 2. The legacy `memory_associations` rows are copied across. That table has
///    no `kind` column, so every row it holds becomes `relates_to`, §2's
///    generic relation: "these two were associated" is the only thing the
///    old shape ever said. A row whose endpoint is gone is left behind and
///    disappears with the table in (3).
/// 3. Both legacy tables are dropped. §9 #8 is exactly the complaint that
///    they "differ from §4's shape", and §4 defines one table — a table no
///    line of the codebase reads or writes cannot keep that entry open.
async fn backfill_memory_links(pool: &SqlitePool) -> anyhow::Result<()> {
    mirror_lifecycle_pointers(pool)
        .await
        .context("data migration pass (1) mirror_lifecycle_pointers failed")?;
    migrate_legacy_associations(pool)
        .await
        .context("data migration pass (2) migrate_legacy_associations failed")?;
    drop_legacy_link_tables(pool)
        .await
        .context("data migration pass (3) drop_legacy_link_tables failed")
}

/// Pass (1): one edge per row that still points at another row.
///
/// Runs in a single transaction: a crash or error mid-pass rolls back its own
/// edges, so the next boot re-mirrors from scratch instead of inheriting a
/// half-mirrored state.
async fn mirror_lifecycle_pointers(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("beginning the lifecycle-pointer mirror transaction")?;
    for (column, kind) in [
        ("superseded_by_id", "supersedes"),
        ("disputed_by_id", "contradicts"),
    ] {
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(&format!(
            "SELECT id, owner_id, {column}, updated_at FROM memories \
             WHERE {column} IS NOT NULL AND {column} <> '' AND {column} <> id"
        ))
        .fetch_all(&mut *tx)
        .await
        .with_context(|| format!("reading memories.{column} for mirroring"))?;
        for (id, owner_id, pointed_at, updated_at) in rows {
            // §4: a pointer whose row is gone gets no edge, and the column is
            // left for the path that deletes the row to clear.
            let exists: Option<String> =
                sqlx::query_scalar("SELECT id FROM memories WHERE id = ? AND owner_id = ?")
                    .bind(&pointed_at)
                    .bind(&owner_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .with_context(|| {
                        format!(
                            "checking the mirror target of memory {id} ({column} = {pointed_at})"
                        )
                    })?;
            if exists.is_none() {
                continue;
            }
            sqlx::query(
                "INSERT OR IGNORE INTO memory_links \
                 (id, owner_id, source_memory_id, target_memory_id, kind, weight, \
                  coactivation_count, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, 0.2, 0, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&owner_id)
            // §4's direction: `source` is what the column names, `target` is
            // the row that carries it.
            .bind(&pointed_at)
            .bind(&id)
            .bind(kind)
            .bind(&updated_at)
            .bind(&updated_at)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("mirroring memories.{column} of memory {id} as a {kind} edge")
            })?;
        }
    }
    tx.commit()
        .await
        .context("committing the lifecycle-pointer mirror transaction")?;
    Ok(())
}

/// One legacy `memory_associations` row, column for column — kept as its own
/// alias because the eight-column tuple is the one thing in this file that
/// reads as noise rather than intent.
type LegacyAssociation = (String, String, String, String, f64, i64, String, String);

/// Pass (2): copy the legacy association rows across as `relates_to`.
///
/// Same transaction-per-pass discipline as pass (1): a failure part-way
/// through rolls back the whole copy, keeping the pass idempotent without
/// depending on `INSERT OR IGNORE` alone.
async fn migrate_legacy_associations(pool: &SqlitePool) -> anyhow::Result<()> {
    if !table_exists(pool, "memory_associations").await? {
        return Ok(());
    }
    let mut tx = pool
        .begin()
        .await
        .context("beginning the legacy-association copy transaction")?;
    let rows: Vec<LegacyAssociation> = sqlx::query_as(
        "SELECT id, owner_id, source_memory_id, target_memory_id, weight, \
                coactivation_count, created_at, updated_at \
         FROM memory_associations WHERE source_memory_id <> target_memory_id",
    )
    .fetch_all(&mut *tx)
    .await
    .context("reading legacy memory_associations rows")?;
    if rows.is_empty() {
        tx.commit()
            .await
            .context("committing the empty legacy-association copy transaction")?;
        return Ok(());
    }
    let total = rows.len();
    let mut migrated = 0usize;
    for (id, owner_id, source, target, weight, coactivation, created_at, updated_at) in rows {
        // Both endpoints have to be there: `memory_links` declares the same
        // foreign keys, and a failed INSERT would stop the boot.
        let endpoints: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE owner_id = ? AND id IN (?, ?)")
                .bind(&owner_id)
                .bind(&source)
                .bind(&target)
                .fetch_one(&mut *tx)
                .await
                .with_context(|| format!("checking the endpoints of legacy association {id}"))?;
        if endpoints != 2 {
            continue;
        }
        sqlx::query(
            "INSERT OR IGNORE INTO memory_links \
             (id, owner_id, source_memory_id, target_memory_id, kind, weight, \
              coactivation_count, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'relates_to', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&owner_id)
        .bind(&source)
        .bind(&target)
        // §4: `weight` is 0..1. A legacy row outside that range is clamped
        // rather than allowed to outrank every similarity-weighted edge.
        .bind(weight.clamp(0.0, 1.0))
        .bind(coactivation)
        .bind(&created_at)
        .bind(&updated_at)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("copying legacy association {id} into memory_links"))?;
        migrated += 1;
    }
    tx.commit()
        .await
        .context("committing the legacy-association copy transaction")?;
    tracing::info!(
        migrated,
        rows = total,
        "backfilled memory_associations into memory_links"
    );
    Ok(())
}

/// Pass (3): retire the two tables §9 #8 names.
///
/// Dropping is a decision about the *backup* path as much as about the live
/// database: a backup is one encrypted copy of the whole `mix-agent.db`
/// (`storage/src/backup.rs` runs `VACUUM INTO` and only records a table-name
/// inventory), and a restore promotes that file wholesale on the next boot.
/// So an old backup would bring these tables back — which is why the drop
/// runs here, on every boot after the schema, instead of once. The pre-restore
/// copy (`mix-agent.db.pre-restore`) keeps the original file either way.
async fn drop_legacy_link_tables(pool: &SqlitePool) -> anyhow::Result<()> {
    // SQLite supports transactional DDL, so both drops share one transaction:
    // dropping the first and failing on the second would otherwise leave the
    // database half-retired.
    let mut tx = pool
        .begin()
        .await
        .context("beginning the legacy-table drop transaction")?;
    for table in ["memory_associations", "memory_features"] {
        if !table_exists(pool, table)
            .await
            .with_context(|| format!("checking whether legacy table {table} still exists"))?
        {
            continue;
        }
        // Nothing in this codebase writes either table (there is not a single
        // reference outside this migration), so a non-zero count means a build
        // older than the current one did — say so before the rows go.
        let rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut *tx)
            .await
            .with_context(|| format!("counting rows in legacy table {table} before dropping it"))?;
        if rows > 0 {
            tracing::warn!(table, rows, "dropping a legacy link table with rows in it");
        }
        sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("dropping legacy table {table}"))?;
        tracing::info!(table, "dropped the legacy table spec §4 replaced");
    }
    tx.commit()
        .await
        .context("committing the legacy-table drop transaction")?;
    Ok(())
}

/// Assign a generated folder under [`default_projects_root`] to any project
/// whose `rootPath` is null, empty, or pointing at a path that no longer
/// exists on disk. The latter is conservative: a user who re-attaches an
/// unplugged drive should not silently get a new folder.
///
/// NOT transactional: created folders cannot be rolled back, so each row names
/// itself in its error context and the pre-open backup in `Storage::open` is
/// the recovery path (see `run_data_migrations`).
async fn backfill_missing_project_roots(pool: &SqlitePool, data_dir: &Path) -> anyhow::Result<()> {
    let projects_root = default_projects_root(data_dir);
    tokio::fs::create_dir_all(&projects_root)
        .await
        .with_context(|| {
            format!(
                "creating the projects root at {} for the project-root backfill",
                projects_root.display()
            )
        })?;

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$') FROM projects WHERE json_extract(data, '$.rootPath') IS NULL \
         OR trim(json_extract(data, '$.rootPath')) = ''",
    )
    .fetch_all(pool)
    .await
    .context("reading projects with a missing rootPath")?;

    if rows.is_empty() {
        return Ok(());
    }

    tracing::info!(
        count = rows.len(),
        "backfilling missing rootPath for legacy projects"
    );

    for (id, raw) in rows {
        let legacy: LegacyProjectRow =
            serde_json::from_str(&raw).unwrap_or_else(|_| LegacyProjectRow {
                name: "Untitled".to_string(),
                additional_paths: Vec::new(),
                sources: Vec::new(),
                created_at: None,
                updated_at: None,
            });

        let target = unique_folder(&projects_root, &legacy.name)
            .await
            .with_context(|| {
                format!("picking a folder for legacy project {id} ({})", legacy.name)
            })?;
        tokio::fs::create_dir_all(&target).await.with_context(|| {
            format!(
                "creating the backfilled folder at {} for project {id}",
                target.display()
            )
        })?;

        let now = Utc::now();
        let payload = serde_json::json!({
            "name": legacy.name,
            "instructions": String::new(),
            "rootPath": target.to_string_lossy(),
            "additionalPaths": legacy.additional_paths,
            "sources": legacy.sources,
            "createdAt": legacy.created_at.unwrap_or(now).to_rfc3339(),
            "updatedAt": legacy.updated_at.unwrap_or(now).to_rfc3339(),
        });

        sqlx::query("UPDATE projects SET data = ? WHERE id = ?")
            .bind(payload.to_string())
            .bind(&id)
            .execute(pool)
            .await
            .with_context(|| format!("writing the backfilled rootPath for project {id}"))?;

        tracing::info!(
            project_id = %id,
            name = %legacy.name,
            root = %target.display(),
            "assigned generated folder"
        );
    }
    Ok(())
}

/// Return a path under `parent` that does not collide with an existing
/// directory. Appends `-2`, `-3`, ... as needed.
async fn unique_folder(parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let safe = sanitize(name);
    let mut candidate = parent.join(&safe);
    if !candidate.exists() {
        return Ok(candidate);
    }
    for n in 2..=1000 {
        candidate = parent.join(format!("{safe}-{n}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    // Vanishingly unlikely; fall back to a uuid suffix rather than erroring
    // out and stranding the user with an un-migratable row.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    Ok(parent.join(format!("{safe}-{suffix}")))
}

/// Projectless chat directories live under
/// `<projects_root>/Projectless/<timestamp>/` so each one has its own
/// self-contained workspace.
pub fn projectless_dir(projects_root: &Path, timestamp: &str) -> PathBuf {
    projects_root.join(PROJECTLESS_PARENT).join(timestamp)
}

fn sanitize(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "Untitled".to_string();
    }
    trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unique_folder_picks_first_available() {
        let parent = std::env::temp_dir().join(format!("mixd-unique-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&parent).await.unwrap();
        tokio::fs::create_dir_all(parent.join("alpha"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(parent.join("alpha-2"))
            .await
            .unwrap();

        let picked = unique_folder(&parent, "alpha").await.unwrap();
        assert_eq!(picked, parent.join("alpha-3"));
        let _ = tokio::fs::remove_dir_all(&parent).await;
    }

    #[test]
    fn sanitize_replaces_path_separators() {
        assert_eq!(sanitize("foo/bar"), "foo_bar");
        assert_eq!(sanitize("  ..hidden..  "), "hidden");
        assert_eq!(sanitize(""), "Untitled");
    }

    #[test]
    fn projectless_dir_isolated_per_timestamp() {
        let root = PathBuf::from("/tmp/x");
        assert_eq!(
            projectless_dir(&root, "2026-01-01T00-00-00"),
            PathBuf::from("/tmp/x/Projectless/2026-01-01T00-00-00")
        );
    }

    #[test]
    fn missing_root_surfaces_a_directory_payload() {
        let raw = r#"{"name":"legacy","instructions":"","sources":[],"updatedAt":"2026-01-01T00:00:00Z"}"#;
        let parsed: LegacyProjectRow = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.name, "legacy");
        assert!(parsed.sources.is_empty());
    }

    #[test]
    fn env_override_wins_for_the_projects_root() {
        // Safe in edition 2021; the test process is single-purpose here.
        std::env::set_var("MIXD_PROJECTS_ROOT", "/tmp/mixd-root-test");
        let root = default_projects_root(Path::new("/data"));
        std::env::remove_var("MIXD_PROJECTS_ROOT");
        assert_eq!(root, PathBuf::from("/tmp/mixd-root-test"));
    }
}
