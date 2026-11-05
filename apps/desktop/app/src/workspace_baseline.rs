//! Git-backed workspace snapshots for "revert this Run".
//!
//! The in-memory checkpoints owned by the engine can only restore files that
//! the `edit` / `files` tools wrote: they never see a rename, a generated file
//! or anything a `terminal` command changed. This module captures the tracked
//! worktree (and the set of untracked paths) before a Run starts, anchored to a
//! ref so it survives restarts, and restores it on request.
//!
//! Limits, stated in the UI rather than hidden here:
//! * the snapshot is a Git stash commit, so pre-existing untracked files are
//!   recorded by path only (their bytes are not copied);
//! * reverting restores the whole worktree to the snapshot, including edits the
//!   user made after the Run finished. The confirmation dialog says so.
//!
//! A non-Git workspace simply returns `None`; the engine's file checkpoints
//! remain the fallback and the UI keeps its old wording.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::cmd_err;

const GIT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_UNTRACKED: usize = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceBaseline {
    pub root: String,
    /// Commit-ish holding the worktree at capture time. Either a stash commit
    /// (`git stash create`) or `HEAD` when the tree was clean.
    pub base: String,
    /// Paths that were untracked at capture time. New untracked paths at revert
    /// time are removed; tracked modifications are restored from `base`.
    pub untracked: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceChange {
    pub path: String,
    /// `M` | `A` | `D` | `?`
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceChanges {
    pub available: bool,
    pub files: Vec<WorkspaceChange>,
}

/// Run a git subcommand in `root`, returning stdout or a short error.
async fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("git {} timed out", args.join(" ")))?
        .map_err(|err| format!("could not run git: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn within_root(root: &Path, raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return path.starts_with(root).then(|| path.to_path_buf());
    }
    let joined = root.join(path);
    // `..` in a git path is unusual but not impossible; reject instead of
    // normalising a path that could escape the workspace.
    if joined
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(joined)
}

async fn untracked(root: &Path) -> Vec<String> {
    let Ok(raw) = git(root, &["ls-files", "--others", "--exclude-standard", "-z"]).await else {
        return Vec::new();
    };
    raw.split('\0')
        .filter(|path| !path.is_empty())
        .take(MAX_UNTRACKED)
        .map(str::to_string)
        .collect()
}

/// Capture the current worktree and anchor the snapshot in a ref.
pub async fn capture(root: Option<&str>, run_id: uuid::Uuid) -> Option<WorkspaceBaseline> {
    let root = PathBuf::from(root?);
    let root = root.canonicalize().ok()?;
    if !root.is_dir() {
        return None;
    }
    let inside = git(&root, &["rev-parse", "--is-inside-work-tree"])
        .await
        .ok()?
        .trim()
        .to_string();
    if inside != "true" {
        return None;
    }

    let stash = git(&root, &["stash", "create"]).await.ok();
    // No changes at capture time leaves this None; HEAD is the baseline then.
    let base = stash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let base = match base {
        Some(base) => base,
        None => git(&root, &["rev-parse", "HEAD"])
            .await
            .ok()?
            .trim()
            .to_string(),
    };
    // Anchor the stash commit. Without a ref, `git gc` is free to prune a
    // dangling commit and the Run silently loses its rollback point.
    let ref_name = format!("refs/mix-agent/runs/{run_id}");
    let _ = git(&root, &["update-ref", &ref_name, &base]).await;
    // A Run that was never reverted and later deleted used to pin its stash
    // commit forever. Prune old refs before returning; one week is far longer
    // than any supported rollback window.
    prune_old_refs(&root, 7 * 24 * 60 * 60).await;

    Some(WorkspaceBaseline {
        root: root.to_string_lossy().into_owned(),
        base,
        untracked: untracked(&root).await,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Delete refs under `refs/mix-agent/runs` older than `keep_seconds`.
///
/// A ref keeps its stash commit alive across `git gc`, which is exactly what
/// makes rollback survive a restart — but an abandoned ref would keep every
/// old worktree snapshot forever. Runs normally live for minutes, so a week is
/// a safe cutoff.
async fn prune_old_refs(root: &Path, keep_seconds: i64) {
    let Ok(raw) = git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname) %(committerdate:unix)",
            "refs/mix-agent/runs",
        ],
    )
    .await
    else {
        return;
    };
    let cutoff = chrono::Utc::now().timestamp() - keep_seconds;
    for line in raw.lines() {
        let Some((ref_name, stamp)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(stamp) = stamp.parse::<i64>() else {
            continue;
        };
        if stamp < cutoff {
            let _ = git(root, &["update-ref", "-d", ref_name]).await;
        }
    }
}

/// Drop the rollback ref before a Run row is deleted.
///
/// The caller has the `runs` row but not its decoded baseline, so this reads
/// the JSON through the same connection first. Failure is logged only: losing
/// a ref is recoverable (the commit is still in the object database until GC),
/// while aborting a user's delete is not.
pub async fn forget_ref(conn: &mut sqlx::SqliteConnection, run_id: &str) {
    let raw: Option<Option<String>> = sqlx::query_scalar(
        "SELECT json_extract(data, '$.workspaceBaseline') FROM runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_optional(&mut *conn)
    .await
    .unwrap_or(None);
    let Some(Some(raw)) = raw else {
        return;
    };
    let Ok(baseline) = serde_json::from_str::<WorkspaceBaseline>(&raw) else {
        return;
    };
    let root = PathBuf::from(&baseline.root);
    let parsed = uuid::Uuid::parse_str(run_id).ok();
    if let Some(run_id) = parsed {
        let ref_name = format!("refs/mix-agent/runs/{run_id}");
        let _ = git(&root, &["update-ref", "-d", &ref_name]).await;
    }
}

/// Capture once, then store the baseline in the Run's JSON row.
pub async fn capture_and_store(pool: &SqlitePool, root: Option<&str>, run_id: uuid::Uuid) {
    // Resume calls this again; the original snapshot must win.
    let existing: Option<(Option<String>,)> =
        sqlx::query_as("SELECT json_extract(data, '$.workspaceBaseline') FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    if existing
        .and_then(|(value,)| value)
        .is_some_and(|value| !value.is_empty())
    {
        return;
    }
    let Some(baseline) = capture(root, run_id).await else {
        return;
    };
    let json = match serde_json::to_string(&baseline) {
        Ok(json) => json,
        Err(err) => {
            tracing::warn!(%err, %run_id, "could not serialise the workspace baseline");
            return;
        }
    };
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE runs SET data = json_set(data, '$.workspaceBaseline', json(?)) WHERE id = ?",
        )
        .bind(json)
        .bind(run_id.to_string())
        .execute(pool)
        .await,
        "store workspace baseline",
    );
}

pub async fn load(pool: &SqlitePool, run_id: uuid::Uuid) -> Option<WorkspaceBaseline> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT json_extract(data, '$.workspaceBaseline') FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let raw = row?.0?;
    serde_json::from_str(&raw).ok()
}

/// Parse `git diff --name-status -z`.
///
/// Most statuses are `STATUS\0path\0`, but rename and copy are three fields:
/// `R100\0old\0new\0` / `C100\0old\0new\0`. The old pairwise parser consumed
/// `old` as the status of the next entry, so every path after a rename shifted
/// and the new path disappeared from the rollback list.
///
/// Rename is represented relative to the baseline as `old: D` + `new: A`;
/// copy only adds `new: A` because its source still exists. That keeps the
/// revert logic a simple per-path restore/remove.
fn parse_name_status(raw: &str) -> Vec<WorkspaceChange> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0');
    while let Some(status) = parts.next() {
        if status.is_empty() {
            continue;
        }
        let code = status.chars().next().unwrap_or('M');
        if matches!(code, 'R' | 'C') {
            let (Some(old), Some(new)) = (parts.next(), parts.next()) else {
                break;
            };
            if code == 'R' && !old.is_empty() {
                out.push(WorkspaceChange {
                    path: old.to_string(),
                    status: "D".to_string(),
                });
            }
            if !new.is_empty() {
                out.push(WorkspaceChange {
                    path: new.to_string(),
                    status: "A".to_string(),
                });
            }
            continue;
        }
        let Some(path) = parts.next() else {
            break;
        };
        if path.is_empty() {
            continue;
        }
        let status = match code {
            'A' => "A",
            'D' => "D",
            // `M`, `T` and unmerged (`U`) paths are restored from the base.
            _ => "M",
        };
        out.push(WorkspaceChange {
            path: path.to_string(),
            status: status.to_string(),
        });
    }
    out
}

/// Paths that differ from the baseline, plus new untracked files.
pub async fn changes(pool: &SqlitePool, run_id: uuid::Uuid) -> Result<WorkspaceChanges, String> {
    let Some(baseline) = load(pool, run_id).await else {
        return Ok(WorkspaceChanges {
            available: false,
            files: Vec::new(),
        });
    };
    let root = PathBuf::from(&baseline.root);
    if !root.is_dir() {
        return Ok(WorkspaceChanges {
            available: false,
            files: Vec::new(),
        });
    }
    let mut files = Vec::new();
    if let Ok(raw) = git(&root, &["diff", "--name-status", "-z", &baseline.base]).await {
        files.extend(parse_name_status(&raw));
    }
    let before: std::collections::HashSet<&str> =
        baseline.untracked.iter().map(String::as_str).collect();
    for path in untracked(&root).await {
        if !before.contains(path.as_str()) {
            files.push(WorkspaceChange {
                path,
                status: "?".to_string(),
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    Ok(WorkspaceChanges {
        available: true,
        files,
    })
}

/// Restore the worktree to the baseline. Returns the number of paths touched.
pub async fn revert(pool: &SqlitePool, run_id: uuid::Uuid) -> Result<usize, String> {
    let Some(baseline) = load(pool, run_id).await else {
        return Ok(0);
    };
    let root = PathBuf::from(&baseline.root);
    if !root.is_dir() {
        return Err(format!(
            "the workspace folder no longer exists: {}",
            baseline.root
        ));
    }
    let changed = changes(pool, run_id).await?;
    let mut touched = 0usize;
    for file in changed.files {
        let Some(path) = within_root(&root, &file.path) else {
            tracing::warn!(path = %file.path, "skipping a rollback path outside the workspace");
            continue;
        };
        if file.status == "?" {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => touched += 1,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::warn!(?err, path = %path.display(), "could not remove a generated file");
                }
            }
            continue;
        }
        // Restore both the worktree and the index so a later `git diff --cached`
        // does not still show the reverted change as staged.
        match git(
            &root,
            &[
                "restore",
                "--source",
                &baseline.base,
                "--staged",
                "--worktree",
                "--",
                &file.path,
            ],
        )
        .await
        {
            Ok(_) => touched += 1,
            Err(err) => {
                // A path that did not exist at the baseline cannot be restored;
                // remove it instead (the `A` case in `--name-status`).
                if file.status == "A" {
                    if tokio::fs::remove_file(&path).await.is_ok() {
                        touched += 1;
                    }
                } else {
                    tracing::warn!(%err, path = %file.path, "git restore failed during rollback");
                }
            }
        }
    }
    let ref_name = format!("refs/mix-agent/runs/{run_id}");
    let _ = git(&root, &["update-ref", "-d", &ref_name]).await;
    mixd_storage::log_write(
        sqlx::query("UPDATE runs SET data = json_remove(data, '$.workspaceBaseline') WHERE id = ?")
            .bind(run_id.to_string())
            .execute(pool)
            .await,
        "clear workspace baseline",
    );
    Ok(touched)
}

/// Parse a run id with the same error shape as the other commands.
pub fn parse_run_id(raw: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(raw).map_err(cmd_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(raw: &str) -> Vec<(String, String)> {
        parse_name_status(raw)
            .into_iter()
            .map(|change| (change.status, change.path))
            .collect()
    }

    #[test]
    fn parses_ordinary_statuses() {
        assert_eq!(
            pairs("M\0src/a.rs\0A\0src/b.rs\0D\0old.txt\0"),
            vec![
                ("M".to_string(), "src/a.rs".to_string()),
                ("A".to_string(), "src/b.rs".to_string()),
                ("D".to_string(), "old.txt".to_string()),
            ]
        );
    }

    #[test]
    fn a_rename_becomes_delete_plus_add() {
        // Git emits three NUL-separated fields for a rename. The old parser
        // consumed `old.txt` as a status and lost `new.txt`.
        assert_eq!(
            pairs("R100\0old.txt\0new.txt\0M\0keep.txt\0"),
            vec![
                ("D".to_string(), "old.txt".to_string()),
                ("A".to_string(), "new.txt".to_string()),
                ("M".to_string(), "keep.txt".to_string()),
            ]
        );
    }

    #[test]
    fn a_copy_adds_only_the_destination() {
        assert_eq!(
            pairs("C85\0base.txt\0copy.txt\0"),
            vec![("A".to_string(), "copy.txt".to_string())]
        );
    }

    #[test]
    fn paths_keep_spaces_and_unicode() {
        assert_eq!(
            pairs("M\0docs/改善 点.md\0"),
            vec![("M".to_string(), "docs/改善 点.md".to_string())]
        );
    }

    #[test]
    fn a_truncated_rename_does_not_lose_the_preceding_entries() {
        assert_eq!(
            pairs("M\0a.txt\0R100\0only-old.txt"),
            vec![("M".to_string(), "a.txt".to_string())]
        );
    }
}
