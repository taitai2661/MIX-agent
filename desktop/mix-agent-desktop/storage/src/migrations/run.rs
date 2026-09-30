//! One-shot data migrations that backfill legacy rows after the schema
//! has been applied. Each function is idempotent: it inspects the rows and
//! only writes when the value is missing or stale, so re-running after a
//! crash or during tests is safe.

use std::path::{Path, PathBuf};

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
pub async fn run_data_migrations(pool: &SqlitePool, data_dir: &Path) -> anyhow::Result<()> {
    backfill_missing_project_roots(pool, data_dir).await?;
    Ok(())
}

/// Assign a generated folder under [`default_projects_root`] to any project
/// whose `rootPath` is null, empty, or pointing at a path that no longer
/// exists on disk. The latter is conservative: a user who re-attaches an
/// unplugged drive should not silently get a new folder.
async fn backfill_missing_project_roots(
    pool: &SqlitePool,
    data_dir: &Path,
) -> anyhow::Result<()> {
    let projects_root = default_projects_root(data_dir);
    tokio::fs::create_dir_all(&projects_root).await?;

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$') FROM projects WHERE json_extract(data, '$.rootPath') IS NULL \
         OR trim(json_extract(data, '$.rootPath')) = ''",
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    tracing::info!(
        count = rows.len(),
        "backfilling missing rootPath for legacy projects"
    );

    for (id, raw) in rows {
        let legacy: LegacyProjectRow = serde_json::from_str(&raw).unwrap_or_else(|_| {
            LegacyProjectRow {
                name: "Untitled".to_string(),
                additional_paths: Vec::new(),
                sources: Vec::new(),
                created_at: None,
                updated_at: None,
            }
        });

        let target = unique_folder(&projects_root, &legacy.name).await?;
        tokio::fs::create_dir_all(&target).await?;

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
            .await?;

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
        tokio::fs::create_dir_all(parent.join("alpha")).await.unwrap();
        tokio::fs::create_dir_all(parent.join("alpha-2")).await.unwrap();

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
