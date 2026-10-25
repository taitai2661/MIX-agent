//! Workspace-scoped file operations.
//!
//! Mirrors the Docker edition's `parent_fd` / `O_NOFOLLOW` semantics: every
//! path is resolved relative to the workspace root, symlinks are not followed,
//! and writes are atomic (write to a temp file, then rename).
//!
//! Reads are line-addressable so an agent can page through a large file instead
//! of pulling the whole thing into the context window. Bulk editing belongs to
//! the `edit` tool; `write` stays here for whole-file replacement.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Tool, ToolContext, ToolResult};

/// Refuse to inline more than this from a single read; return a pointer instead.
pub const MAX_READ_BYTES: usize = 256 * 1024;
/// Default page size when the model asks for a slice without a limit.
const DEFAULT_READ_LINES: usize = 2_000;

#[derive(Default)]
pub struct FilesTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

impl FilesTool {
    pub fn resolve_workspace_root(workspace_root: &Path) -> anyhow::Result<PathBuf> {
        if !workspace_root.exists() {
            std::fs::create_dir_all(workspace_root)?;
        }
        Ok(workspace_root.canonicalize()?)
    }

    /// Resolve the root a run is scoped to: the project's folder when set,
    /// otherwise the process working directory (legacy behaviour).
    pub fn resolve_context_root(root_path: Option<&str>) -> anyhow::Result<PathBuf> {
        match root_path {
            Some(root) => {
                let path = PathBuf::from(root);
                if !path.is_dir() {
                    anyhow::bail!("workspace root not found: {}", root);
                }
                Ok(path.canonicalize()?)
            }
            None => std::env::current_dir()?.canonicalize().map_err(Into::into),
        }
    }

    /// Resolve `path` for a tool call against the run's folders.
    ///
    /// * Relative paths resolve inside the writable project root.
    /// * Absolute paths must land inside the project root (writable) or one
    ///   of `additional` roots (read-only, Codex `--add-dir` parity); they
    ///   are rejected otherwise so a stray `/etc/passwd` cannot be read.
    ///
    /// Returns the resolved path and whether writes are permitted there.
    pub fn resolve_target(
        root_path: Option<&str>,
        additional: &[String],
        path: &str,
    ) -> anyhow::Result<(PathBuf, bool)> {
        let main = Self::resolve_context_root(root_path)?;
        Self::resolve_target_in(&main, additional, path)
    }

    /// [`Self::resolve_target`] with the canonical writable root already in
    /// hand, so callers that need the root too do not canonicalize twice.
    pub fn resolve_target_in(
        main: &Path,
        additional: &[String],
        path: &str,
    ) -> anyhow::Result<(PathBuf, bool)> {
        if Path::new(path).is_absolute() {
            // The leaf may not exist yet (a `write` creating it), so resolve
            // the deepest existing ancestor and re-append the rest.
            let canon = canonicalize_partial(Path::new(path));
            if canon.starts_with(main) {
                return Ok((canon, true));
            }
            for extra in additional {
                let trimmed = extra.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let extra_path = Path::new(trimmed);
                if !extra_path.is_dir() {
                    continue;
                }
                let extra_canon = extra_path.canonicalize()?;
                if canon.starts_with(&extra_canon) {
                    return Ok((canon, false));
                }
            }
            anyhow::bail!("path is outside the workspace: {path}");
        }
        Ok((Self::safe_join(main, path)?, true))
    }

    /// Lexically drop `.` / resolve `..` so paths that do not exist yet still
    /// cannot escape the root (`Path::canonicalize` fails on them).
    fn lexical_normalize(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    pub fn safe_join(root: &Path, path: &str) -> anyhow::Result<PathBuf> {
        let cleaned = path.trim_start_matches('/');
        let root_canon = root.canonicalize()?;
        let candidate = Self::lexical_normalize(&root_canon.join(cleaned));
        // Resolve component by component. Calling `canonicalize` on the whole
        // path fails whenever the leaf does not exist yet — which is every
        // `write` / `create` — and the lexical fallback that followed let a
        // symlinked directory in the middle of the path escape the workspace.
        let relative = candidate
            .strip_prefix(&root_canon)
            .map_err(|_| anyhow::anyhow!("path escapes workspace: {}", path))?;
        let mut resolved = root_canon.clone();
        for component in relative.components() {
            let next = resolved.join(component.as_os_str());
            match std::fs::symlink_metadata(&next) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    // Fail closed: a symlink is exactly the escape route when
                    // the leaf behind it does not exist yet.
                    let target = next.canonicalize()?;
                    if !target.starts_with(&root_canon) {
                        anyhow::bail!("path escapes workspace: {}", path);
                    }
                    resolved = target;
                }
                Ok(_) => resolved = next,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    // Nothing below this point exists, so nothing below it can
                    // be a symlink either; append the rest lexically.
                    resolved = next;
                }
                Err(err) => return Err(err.into()),
            }
        }
        if !resolved.starts_with(&root_canon) {
            anyhow::bail!("path escapes workspace: {}", path);
        }
        Ok(resolved)
    }
}

/// Canonicalize `path`, falling back to canonicalizing the deepest existing
/// ancestor and re-appending the not-yet-created remainder. Used so a
/// not-yet-written file's absolute path can still be compared against the
/// canonical roots (`/var/...` is a symlink to `/private/var/...` on macOS).
fn canonicalize_partial(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) if !existing.exists() => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut base = existing.canonicalize().unwrap_or(existing);
    for part in rest.into_iter().rev() {
        base.push(part);
    }
    base
}

/// A line-addressed slice of a file, plus enough metadata for the model to know
/// whether it saw everything.
#[derive(Debug, Clone, Serialize)]
pub struct FilePage {
    pub path: String,
    pub content: String,
    /// 1-based line number of the first returned line.
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    /// True when more lines remain after `end_line`.
    pub has_more: bool,
    /// True when the file was too large to inline at all.
    pub truncated: bool,
}

/// Read a slice of `path`, paging by 1-based line numbers.
fn read_page(path: &Path, args: &serde_json::Value, display: &str) -> anyhow::Result<FilePage> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        anyhow::bail!("{} is a directory; use action \"list\"", display);
    }
    if meta.len() as usize > MAX_READ_BYTES {
        return Ok(FilePage {
            path: display.to_string(),
            content: String::new(),
            start_line: 0,
            end_line: 0,
            total_lines: 0,
            has_more: false,
            truncated: true,
        });
    }
    let full = std::fs::read_to_string(path)?;
    let all: Vec<&str> = full.lines().collect();
    let total_lines = all.len();
    let start_line = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|n| n.max(1) as usize)
        .unwrap_or(1)
        .min(total_lines.max(1));
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        // A JSON `u64` reaches this as `usize::MAX`, and `start_line - 1 + limit`
        // then wrapped around — the slice below panicked with
        // "slice index starts at … but ends at …".
        .map(|n| n.clamp(1, 100_000) as usize)
        .unwrap_or(DEFAULT_READ_LINES);
    let end_line = (start_line - 1).saturating_add(limit).min(total_lines);
    let slice = if total_lines == 0 || start_line > total_lines {
        String::new()
    } else {
        all[start_line - 1..end_line].join("\n")
    };
    Ok(FilePage {
        path: display.to_string(),
        content: slice,
        start_line: if total_lines == 0 { 0 } else { start_line },
        end_line,
        total_lines,
        has_more: end_line < total_lines,
        truncated: false,
    })
}

/// A unique sibling of `target` used as the staging file for an atomic write.
///
/// The name embeds the full file name (so `README` and `README.md` do not share
/// a temp path) plus a pid and a counter (so two concurrent writes to the *same*
/// target cannot interleave).
pub(crate) fn temp_sibling(target: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(
        ".{}.{}.{}.mix-agent-tmp",
        file_name,
        std::process::id(),
        n
    ))
}

#[async_trait]
impl Tool for FilesTool {
    fn id(&self) -> &str {
        "files"
    }
    fn name(&self) -> &str {
        "Files"
    }
    fn description(&self) -> &str {
        "Read, write, list, search and delete files inside the workspace root. Use `edit` \
         instead of `write` for changes to existing files; `write` replaces a whole file. \
         `read` supports `offset` / `limit` (1-based line numbers) for large files."
    }
    /// Reading is free; changing the workspace is not. Deciding per action
    /// rather than per tool is what lets an agent look at a repository without
    /// a click while still gating every write and delete.
    fn is_sensitive(&self, args: &serde_json::Value) -> bool {
        !matches!(
            args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
            "read" | "list" | "search"
        )
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["read", "write", "list", "search", "delete"] },
                "path": { "type": "string" },
                "content": { "type": "string" },
                "pattern": { "type": "string" },
                "offset": {
                    "type": "integer",
                    "description": "1-based first line to return. Only used by `read`."
                },
                "limit": {
                    "type": "integer",
                    "description": "How many lines to return. Only used by `read`."
                }
            },
            "required": ["action", "path"]
        })
    }
    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        // Every branch below is synchronous filesystem work — including the
        // `search` walk, which can touch thousands of files — so the whole
        // action is handed to the blocking pool. Doing it inline used to park
        // the async executor (and with it every in-flight Run) on disk I/O.
        //
        // The action and path are captured before the move so a failure can be
        // reported with the file it was about, not just the tool name.
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let root_path = ctx.root_path;
        let additional = ctx.additional_paths;
        match tokio::task::spawn_blocking(move || {
            FilesTool::invoke_blocking(root_path, additional, args)
        })
        .await
        {
            Ok(outcome) => {
                if let Err(err) = &outcome {
                    tracing::warn!(action, path, %err, "file tool failed");
                }
                outcome
            }
            Err(join_err) => {
                tracing::error!(action, path, %join_err, "file tool worker panicked");
                Err(anyhow::anyhow!("file worker failed: {join_err}"))
            }
        }
    }
}

impl FilesTool {
    fn invoke_blocking(
        root_path: Option<String>,
        additional: Vec<String>,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("path required"))?;
        let workspace_root = Self::resolve_context_root(root_path.as_deref())?;
        let (resolved, writable) = Self::resolve_target_in(&workspace_root, &additional, path)?;
        match action {
            "read" => {
                let page = read_page(&resolved, &args, path)?;
                Ok(ToolResult {
                    ok: true,
                    content: serde_json::to_value(page).unwrap_or_else(|_| serde_json::json!({})),
                    error: None,
                })
            }
            "write" => {
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("content required for write"))?;
                if !writable {
                    anyhow::bail!("追加フォルダは読み取り専用です: {path}");
                }
                if resolved == workspace_root {
                    anyhow::bail!("refusing to write over the workspace root");
                }
                if let Some(parent) = resolved.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                // A name unique to this target: `with_extension` maps `README`
                // and `README.md` (and any `a.b` / `a.c` pair) onto the same
                // temp path, so two overlapping writes corrupted each other.
                let tmp = temp_sibling(&resolved);
                if let Err(err) = std::fs::write(&tmp, content) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(err.into());
                }
                if let Err(err) = std::fs::rename(&tmp, &resolved) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(err.into());
                }
                Ok(ToolResult {
                    ok: true,
                    content: serde_json::json!({ "path": path, "bytes": content.len() }),
                    error: None,
                })
            }
            "list" => {
                let mut entries = Vec::new();
                if resolved.is_dir() {
                    for entry in std::fs::read_dir(&resolved)? {
                        let entry = entry?;
                        let meta = entry.metadata()?;
                        entries.push(FileEntry {
                            name: entry.file_name().to_string_lossy().to_string(),
                            is_dir: meta.is_dir(),
                            size: meta.len(),
                        });
                    }
                }
                Ok(ToolResult {
                    ok: true,
                    content: serde_json::json!({ "path": path, "entries": entries }),
                    error: None,
                })
            }
            "search" => {
                // Kept for backwards compatibility with the flat `files` schema.
                // The `grep` tool is the supported entry point: it honours
                // .gitignore, detects binaries and fails loudly on a bad regex.
                let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                let regex = regex::Regex::new(pattern)
                    .map_err(|e| anyhow::anyhow!("invalid regex {pattern:?}: {e}"))?;
                let mut hits = Vec::new();
                let mut truncated = false;
                // Long lines (minified bundles, generated files) must not each
                // inject kilobytes into the model context.
                let clip = |s: &str| -> String {
                    if s.chars().count() <= 500 {
                        s.to_string()
                    } else {
                        let mut out: String = s.chars().take(500).collect();
                        out.push('…');
                        out
                    }
                };
                if resolved.is_dir() {
                    'outer: for entry in super::search::workspace_entries(&resolved) {
                        if entry
                            .metadata()
                            .map(|m| m.len() > super::search::MAX_FILE_BYTES)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                        if super::search::is_probably_binary(entry.path()) {
                            continue;
                        }
                        let Ok(text) = std::fs::read_to_string(entry.path()) else {
                            continue;
                        };
                        for (idx, line) in text.lines().enumerate() {
                            if regex.is_match(line) {
                                let relative = entry
                                    .path()
                                    .strip_prefix(&workspace_root)
                                    .unwrap_or(entry.path())
                                    .display()
                                    .to_string();
                                hits.push(serde_json::json!({
                                    "path": relative,
                                    "line": idx + 1,
                                    "text": clip(line),
                                }));
                                if hits.len() >= 200 {
                                    truncated = true;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
                Ok(ToolResult {
                    ok: true,
                    content: serde_json::json!({
                        "path": path,
                        "hits": hits,
                        "truncated": truncated,
                        "hint": "Use the `grep` tool for new searches; it supports glob filters.",
                    }),
                    error: None,
                })
            }
            "delete" => {
                // `path` of `""`, `"."` or `"/"` resolves to the root itself,
                // and an "always allow" rule for `files` means no prompt — a
                // single call would wipe the whole project.
                if !writable {
                    anyhow::bail!("追加フォルダは読み取り専用です: {path}");
                }
                if resolved == workspace_root {
                    anyhow::bail!("refusing to delete the workspace root");
                }
                if resolved.is_dir() {
                    std::fs::remove_dir_all(&resolved)?;
                } else {
                    std::fs::remove_file(&resolved)?;
                }
                Ok(ToolResult {
                    ok: true,
                    content: serde_json::json!({ "path": path, "deleted": true }),
                    error: None,
                })
            }
            other => anyhow::bail!("unknown action: {}", other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mixd-files-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn safe_join_allows_paths_inside_the_root() {
        let root = temp_root("join-ok");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        let joined = FilesTool::safe_join(&root, "a/b").unwrap();
        assert!(joined.starts_with(&root));
    }

    #[test]
    fn safe_join_blocks_traversal_on_missing_files() {
        // The old implementation canonicalized nothing for nonexistent paths,
        // so `../` slipped through here.
        let root = temp_root("join-missing");
        assert!(FilesTool::safe_join(&root, "../outside.txt").is_err());
        assert!(FilesTool::safe_join(&root, "a/../../outside.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn safe_join_blocks_symlink_escape() {
        let root = temp_root("join-symlink");
        let outside =
            std::env::temp_dir().join(format!("mixd-files-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(FilesTool::safe_join(&root, "link").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn safe_join_blocks_escape_through_a_symlinked_directory_with_a_missing_leaf() {
        // `canonicalize` fails on the leaf because it does not exist yet, which
        // is every `write` / `create`; the lexical fallback that followed let
        // this path through.
        let root = temp_root("join-symlink-missing");
        let outside =
            std::env::temp_dir().join(format!("mixd-files-outside-missing-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(
            FilesTool::safe_join(&root, "link/newfile.txt").is_err(),
            "a new file behind a symlinked directory must not escape the root"
        );
        // A *real* directory under the symlink that points back inside is fine.
        let inside = root.join("real");
        std::fs::create_dir_all(&inside).unwrap();
        let inner_link = root.join("inner");
        std::os::unix::fs::symlink(&inside, &inner_link).unwrap();
        assert!(FilesTool::safe_join(&root, "inner/notes.txt").is_ok());
    }

    #[test]
    fn resolve_context_root_requires_an_existing_folder() {
        assert!(FilesTool::resolve_context_root(None).is_ok());
        assert!(FilesTool::resolve_context_root(Some("/definitely/not/a/dir")).is_err());
    }
}
