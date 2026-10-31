//! Workspace browsing commands for the file tree / review UI.
//!
//! The WebView has no `fs` / `shell` access, so directory listing, file
//! preview and git status all go through these commands. Every path is
//! resolved against the requested project root with the same traversal checks
//! as the `files` Tool.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::AppState;

/// Hard cap per directory listing (defensive against huge folders).
const MAX_DIR_ENTRIES: usize = 2_000;
/// Hard cap for a previewed file.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Cap for git diff output handed to the UI.
const MAX_DIFF_BYTES: usize = 512 * 1024;
/// Wall-clock limit for git subprocesses.
const GIT_TIMEOUT_MS: u64 = 10_000;

/// Directory names hidden from the tree by default (always-hidden noise).
const HIDDEN_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
];

async fn resolve_root(
    app: &AppHandle,
    state: &AppState,
    root_path: &str,
) -> Result<std::path::PathBuf, String> {
    let trimmed = root_path.trim();
    if trimmed.is_empty() {
        return Err("root_path required".to_string());
    }
    let path = std::path::PathBuf::from(trimmed);
    if !path.is_dir() {
        return Err(format!("フォルダが見つかりません: {trimmed}"));
    }
    let root = path.canonicalize().map_err(crate::error::cmd_err)?;
    crate::commands::access::ensure_root(app, state, &root).await?;
    Ok(root)
}

#[derive(Serialize)]
pub struct DirEntryView {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub hidden: bool,
}

/// List one directory level for the file tree. `path` is root-relative;
/// dotfiles are returned with `hidden: true` so the tree can filter them.
#[tauri::command]
pub async fn list_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
    path: String,
) -> Result<Vec<DirEntryView>, String> {
    let root = resolve_root(&app, &state, &root_path).await?;
    let target =
        mixd_tools::files::FilesTool::safe_join(&root, &path).map_err(crate::error::cmd_err)?;
    if !target.is_dir() {
        return Ok(Vec::new());
    }
    let read = tokio::task::spawn_blocking(move || -> Result<Vec<DirEntryView>, String> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&target).map_err(crate::error::cmd_err)? {
            let entry = entry.map_err(crate::error::cmd_err)?;
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            let name = entry.file_name().to_string_lossy().to_string();
            let hidden = name.starts_with('.') || HIDDEN_DIRS.contains(&name.as_str());
            entries.push(DirEntryView {
                is_dir: meta.is_dir(),
                size: meta.len(),
                name,
                hidden,
            });
            if entries.len() >= MAX_DIR_ENTRIES {
                break;
            }
        }
        entries.sort_by(|a, b| {
            (b.is_dir, a.name.to_lowercase()).cmp(&(a.is_dir, b.name.to_lowercase()))
        });
        Ok(entries)
    })
    .await
    .map_err(crate::error::cmd_err)??;
    Ok(read)
}

#[derive(Serialize)]
pub struct FileContentView {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    pub is_binary: bool,
}

/// Read a project file for the read-only preview pane.
#[tauri::command]
pub async fn read_workspace_file(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
    path: String,
) -> Result<FileContentView, String> {
    let root = resolve_root(&app, &state, &root_path).await?;
    let target =
        mixd_tools::files::FilesTool::safe_join(&root, &path).map_err(crate::error::cmd_err)?;
    if !target.is_file() {
        return Err(format!("ファイルが見つかりません: {path}"));
    }
    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(crate::error::cmd_err)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "ファイルが大きすぎます（{} KB / 上限 1024 KB）",
            meta.len() / 1024
        ));
    }
    let bytes = tokio::fs::read(&target)
        .await
        .map_err(crate::error::cmd_err)?;
    match String::from_utf8(bytes) {
        Ok(content) => Ok(FileContentView {
            path,
            content,
            truncated: false,
            is_binary: false,
        }),
        Err(err) => {
            if looks_binary(err.as_bytes()) {
                Ok(FileContentView {
                    path,
                    content: String::new(),
                    truncated: false,
                    is_binary: true,
                })
            } else {
                Err("UTF-8 として読み込めません".to_string())
            }
        }
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8_000).any(|b| *b == 0)
}

#[derive(Serialize)]
pub struct GitStatusEntry {
    /// Root-relative path (renames show the new path).
    pub path: String,
    /// Single-letter XY status from `git status --porcelain`.
    pub status: String,
}

#[derive(Serialize)]
pub struct GitStatusView {
    /// False when git is missing or the folder is not a repository.
    pub available: bool,
    pub branch: Option<String>,
    pub entries: Vec<GitStatusEntry>,
}

async fn run_git(root: &std::path::Path, args: &[&str]) -> Result<std::process::Output, String> {
    // Same `kill_on_drop(true)` + `timeout` shape as the tools-side `run_once`
    // (`tools/src/terminal.rs`): a hung git must not outlive the call. Keep
    // the two in step; see the cross-reference there.
    use tokio::process::Command;
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    tokio::time::timeout(
        std::time::Duration::from_millis(GIT_TIMEOUT_MS),
        cmd.output(),
    )
    .await
    .map_err(|_| "git timed out".to_string())?
    .map_err(crate::error::cmd_err)
}

/// `git status --porcelain` for the file tree decorations.
#[tauri::command]
pub async fn git_status(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
) -> Result<GitStatusView, String> {
    let root = resolve_root(&app, &state, &root_path).await?;
    let output = match run_git(&root, &["status", "--porcelain=v1", "-uall"]).await {
        Ok(output) => output,
        Err(_) => {
            return Ok(GitStatusView {
                available: false,
                branch: None,
                entries: Vec::new(),
            })
        }
    };
    if !output.status.success() {
        // Not a repository (exit 128) or similar → git UI stays hidden.
        return Ok(GitStatusView {
            available: false,
            branch: None,
            entries: Vec::new(),
        });
    }
    let branch = run_git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        // `split_at(2)` panics when byte 2 is not a UTF-8 boundary, which a
        // mangled path in `git status` can produce. `get` returns `None`
        // instead, so an unparseable line is skipped rather than crashing.
        let Some(xy) = line.get(..2) else {
            continue;
        };
        let rest = line.get(2..).unwrap_or("").trim_start();
        let path = if let Some((_old, new)) = rest.split_once(" -> ") {
            // Rename: prefer the new path for display.
            new.trim().to_string()
        } else {
            rest.to_string()
        };
        if path.is_empty() {
            continue;
        }
        entries.push(GitStatusEntry {
            path,
            status: xy.trim().to_string(),
        });
    }
    Ok(GitStatusView {
        available: true,
        branch,
        entries,
    })
}

#[derive(Serialize)]
pub struct GitDiffView {
    pub available: bool,
    pub scope: String,
    pub unified_diff: String,
    pub truncated: bool,
}

/// Unified diff of the working tree (`unstaged`), the index (`staged`) or a
/// single file (`scope = "file"` with `path` set → both stages for that file).
#[tauri::command]
pub async fn git_diff(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
    scope: String,
    path: Option<String>,
) -> Result<GitDiffView, String> {
    let root = resolve_root(&app, &state, &root_path).await?;
    let mut args: Vec<&str> = vec!["diff", "--no-ext-diff", "--no-color"];
    let path = path.filter(|p| !p.trim().is_empty());
    match scope.as_str() {
        "staged" => args.push("--cached"),
        "file" => {}
        // Default: full working tree (unstaged) diff.
        _ => {}
    }
    // Validate the path against traversal before handing it to git.
    if let Some(p) = &path {
        mixd_tools::files::FilesTool::safe_join(&root, p).map_err(crate::error::cmd_err)?;
        args.push("--");
        args.push(p);
    }
    let output = match run_git(&root, &args).await {
        Ok(output) => output,
        Err(_) => {
            return Ok(GitDiffView {
                available: false,
                scope,
                unified_diff: String::new(),
                truncated: false,
            })
        }
    };
    if !output.status.success() {
        return Ok(GitDiffView {
            available: false,
            scope,
            unified_diff: String::from_utf8_lossy(&output.stderr).to_string(),
            truncated: false,
        });
    }
    let mut unified = String::from_utf8_lossy(&output.stdout).to_string();
    let truncated = unified.len() > MAX_DIFF_BYTES;
    if truncated {
        // Shared boundary walk with the terminal tool
        // (`mixd_tools::terminal::truncate_to_char_boundary`): a byte cut
        // inside a multi-byte character panics, and the release profile aborts.
        let cut = mixd_tools::terminal::truncate_to_char_boundary(&unified, MAX_DIFF_BYTES).len();
        unified.truncate(cut);
    }
    Ok(GitDiffView {
        available: true,
        scope,
        unified_diff: unified,
        truncated,
    })
}

/// Find workspace files respecting .gitignore and default exclusion lists.
/// Returns relative paths, up to `limit` entries (default 300).
#[tauri::command]
pub async fn find_workspace_files(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let root = resolve_root(&app, &state, &root_path).await?;
    let max = limit.unwrap_or(300).min(1000);
    let q = query
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());

    let files = tokio::task::spawn_blocking(move || {
        let mut builder = ignore::WalkBuilder::new(&root);
        builder.hidden(true);
        builder.git_ignore(true);
        builder.git_global(true);
        builder.git_exclude(true);
        builder.parents(true);

        for &dir in HIDDEN_DIRS {
            builder.add_custom_ignore_filename(dir);
        }

        let mut results = Vec::new();
        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(&root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().to_string();

            if let Some(ref needle) = q {
                if !rel_str.to_lowercase().contains(needle) {
                    continue;
                }
            }

            results.push(rel_str);
            if results.len() >= max {
                break;
            }
        }
        results
    })
    .await
    .map_err(crate::error::cmd_err)?;

    Ok(files)
}

/// Restore a file's working tree or staged state (git restore / discard).
#[tauri::command]
pub async fn git_restore(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
    path: String,
    staged: bool,
) -> Result<(), String> {
    let root = resolve_root(&app, &state, &root_path).await?;
    mixd_tools::files::FilesTool::safe_join(&root, &path).map_err(crate::error::cmd_err)?;

    let mut args = vec!["restore"];
    if staged {
        args.push("--staged");
    }
    args.push("--");
    args.push(&path);

    let output = run_git(&root, &args)
        .await
        .map_err(|e| format!("git restore failed: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if !staged && err.contains("did not match any file") {
            let target = root.join(&path);
            if target.is_file() {
                let _ = tokio::fs::remove_file(target).await;
                return Ok(());
            }
        }
        return Err(format!("git restore error: {err}"));
    }

    Ok(())
}

/// Stage or unstage a file (git add / git restore --staged).
#[tauri::command]
pub async fn git_stage(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
    path: String,
    unstage: bool,
) -> Result<(), String> {
    let root = resolve_root(&app, &state, &root_path).await?;
    mixd_tools::files::FilesTool::safe_join(&root, &path).map_err(crate::error::cmd_err)?;

    let args: Vec<&str> = if unstage {
        vec!["restore", "--staged", "--", &path]
    } else {
        vec!["add", "--", &path]
    };

    let output = run_git(&root, &args)
        .await
        .map_err(|e| format!("git stage failed: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git stage error: {err}"));
    }

    Ok(())
}
