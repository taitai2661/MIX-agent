//! Workspace-scoped file operations.
//!
//! Mirrors the Docker edition's `parent_fd` / `O_NOFOLLOW` semantics: every
//! path is resolved relative to the workspace root, symlinks are not followed,
//! and writes are atomic (write to a temp file, then rename).
//!
//! Reads are line-addressable so an agent can page through a large file instead
//! of pulling the whole thing into the context window. Bulk editing belongs to
//! the `edit` tool; `write` stays here for whole-file replacement.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Tool, ToolContext, ToolResult};

/// Hard cap on what an explicit `limit` may inline. Larger files are returned
/// as head + tail so one read cannot swallow the context window.
pub const MAX_READ_BYTES: usize = 128 * 1024;
/// Default page size when the model asks for a slice without a limit.
const DEFAULT_READ_LINES: usize = 400;
/// Default byte budget for a read without an explicit `limit`. A single long
/// file would otherwise spend tens of thousands of tokens per call.
const DEFAULT_READ_BYTES: usize = 48 * 1024;
/// How much of a file larger than [`MAX_READ_BYTES`] is shown from each end.
const HEAD_TAIL_BYTES: usize = 48 * 1024;

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
    /// 1-based offset to pass on the next read when `has_more` is true.
    pub next_offset: Option<usize>,
    /// True when the file was too large to inline at all.
    pub truncated: bool,
    /// Stable-within-process fingerprint of the whole file, so the engine can
    /// notice that the content changed between the read and a later edit.
    /// `0` means "not computed" (the file was too large).
    pub fingerprint: u64,
}

/// Read a slice of `path`, paging by 1-based line numbers.
fn read_page(path: &Path, args: &serde_json::Value, display: &str) -> anyhow::Result<FilePage> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        anyhow::bail!("{} is a directory; use action \"list\"", display);
    }
    if meta.len() as usize > MAX_READ_BYTES {
        return read_head_tail(path, display);
    }
    let full = std::fs::read_to_string(path)?;
    let fingerprint = fingerprint_of(full.as_bytes());
    let all: Vec<&str> = full.lines().collect();
    let total_lines = all.len();
    let start_line = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|n| n.max(1) as usize)
        .unwrap_or(1)
        .min(total_lines.max(1));
    let explicit_limit = args.get("limit").and_then(|v| v.as_u64());
    let limit = explicit_limit
        // A JSON `u64` reaches this as `usize::MAX`, and `start_line - 1 + limit`
        // then wrapped around — the slice below panicked with
        // "slice index starts at … but ends at …".
        .map(|n| n.clamp(1, 100_000) as usize)
        .unwrap_or(DEFAULT_READ_LINES);
    // The default page is deliberately small; an explicit `limit` may ask for
    // more, but never past the hard cap that keeps one read from swallowing the
    // context window.
    let byte_budget = if explicit_limit.is_some() {
        MAX_READ_BYTES
    } else {
        DEFAULT_READ_BYTES
    };
    // Prefix every line with its 1-based number. Without it the model has to
    // count lines to use `offset`/`limit`, and a line number quoted from
    // `symbols` could not be turned into a read window at all. The format
    // matches what other coding agents emit, so anchors copied back into
    // `edit` are recognised (see `edit::strip_line_number_prefixes`).
    let mut content = String::new();
    let mut end_line = start_line.saturating_sub(1);
    if total_lines > 0 {
        for (offset, line) in all[start_line - 1..].iter().enumerate() {
            if offset >= limit {
                break;
            }
            let line_no = start_line + offset;
            let formatted = format!("{:>6}\t{}", line_no, line);
            let needed = formatted.len() + usize::from(!content.is_empty());
            if content.len() + needed > byte_budget {
                // Always return at least one line, truncated on a character
                // boundary, so a pathological single-line file still yields
                // something the model can act on.
                if content.is_empty() {
                    let prefix = format!("{:>6}\t", line_no);
                    let remaining = byte_budget.saturating_sub(prefix.len());
                    content.push_str(&prefix);
                    content.push_str(crate::terminal::truncate_to_char_boundary(line, remaining));
                    end_line = line_no;
                }
                break;
            }
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&formatted);
            end_line = line_no;
        }
    }
    let has_more = end_line < total_lines;
    Ok(FilePage {
        path: display.to_string(),
        content,
        start_line: if total_lines == 0 { 0 } else { start_line },
        end_line,
        total_lines,
        has_more,
        next_offset: has_more.then_some(end_line + 1),
        truncated: false,
        fingerprint,
    })
}

/// Return the first and last [`HEAD_TAIL_BYTES`] of a file that is too large to
/// inline. Reading the ends instead of returning nothing keeps the model
/// oriented (imports, module header, trailing definitions) while still telling
/// it, explicitly, that the middle was dropped.
fn read_head_tail(path: &Path, display: &str) -> anyhow::Result<FilePage> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len() as usize;
    let head_len = HEAD_TAIL_BYTES.min(len);
    let tail_len = HEAD_TAIL_BYTES.min(len - head_len);

    let mut head = vec![0u8; head_len];
    file.read_exact(&mut head)?;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0u8; tail_len];
    file.read_exact(&mut tail)?;

    // Drop the partial line at each split so the model does not try to use half
    // a line as an anchor.
    let head = String::from_utf8_lossy(&head);
    let head = head
        .rsplit_once('\n')
        .map(|(before, _)| before)
        .unwrap_or(&head);
    let tail = String::from_utf8_lossy(&tail);
    let tail = tail
        .split_once('\n')
        .map(|(_, after)| after)
        .unwrap_or(&tail);
    let content = format!(
        "{head}\n\n... [truncated: file is {len} bytes; showing the first and last \
         {HEAD_TAIL_BYTES} bytes. Use `terminal` (e.g. sed -n) to read the middle.] ...\n\n{tail}"
    );
    Ok(FilePage {
        path: display.to_string(),
        content,
        start_line: 1,
        end_line: 0,
        total_lines: 0,
        has_more: false,
        next_offset: None,
        truncated: true,
        fingerprint: 0,
    })
}

/// 64-bit SipHash of the file bytes. Only ever compared with another value from
/// this process, so a stable cryptographic hash would be overkill.
pub fn fingerprint_of(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// A unique sibling of `target` used as the staging file for an atomic write.
///
/// The name embeds the full file name (so `README` and `README.md` do not share
/// a temp path) plus a pid and a counter (so two concurrent writes to the *same*
/// target cannot interleave).
pub(crate) fn temp_sibling(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    // Random, not pid+counter: the old names were predictable, so a model
    // could plant a symlink at the staging path with `terminal` and make the
    // following `std::fs::write` write through it. `create_new` below refuses
    // an existing entry, so a planted file or symlink cannot be followed.
    let suffix = uuid::Uuid::new_v4().simple();
    target.with_file_name(format!(".{file_name}.{suffix}.mix-agent-tmp"))
}

/// Atomically replace `target` with `content` without following a leaf symlink.
///
/// The staging file is created with `create_new` and mode 0600, fsynced, then
/// renamed over the target. `rename` replaces a symlink rather than writing
/// through it, and the random name makes pre-planting the staging path
/// impractical.
pub(crate) fn atomic_write_bytes(target: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_sibling(target);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    let written = (|| -> std::io::Result<()> {
        file.write_all(content)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    // Persist the directory entry so a crash cannot resurrect the old name.
    if let Some(parent) = target.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
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
         `read` returns each line prefixed with its 1-based line number and supports \
         `offset` / `limit` (1-based line numbers) for large files. Without a `limit` \
         it returns at most 400 lines / 48 KiB; use `next_offset` to continue."
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
                // Atomic, no-follow write: see `atomic_write_bytes`.
                atomic_write_bytes(&resolved, content.as_bytes())?;
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

    #[test]
    fn read_prefixes_every_line_with_its_number() {
        let root = temp_root("read-numbered");
        let file = root.join("sample.txt");
        std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

        let page = read_page(&file, &serde_json::json!({}), "sample.txt").unwrap();
        assert_eq!(page.content, "     1\talpha\n     2\tbeta\n     3\tgamma");
        assert_eq!(page.start_line, 1);
        assert_eq!(page.end_line, 3);
        assert_eq!(page.total_lines, 3);
        assert!(!page.has_more);
    }

    #[test]
    fn read_offset_keeps_the_original_line_numbers() {
        let root = temp_root("read-offset");
        let file = root.join("sample.txt");
        std::fs::write(&file, "one\ntwo\nthree\nfour\n").unwrap();

        let page = read_page(
            &file,
            &serde_json::json!({ "offset": 2, "limit": 2 }),
            "sample.txt",
        )
        .unwrap();
        assert_eq!(page.content, "     2\ttwo\n     3\tthree");
        assert_eq!(page.start_line, 2);
        assert_eq!(page.end_line, 3);
        assert!(page.has_more);
    }

    #[test]
    fn read_defaults_to_a_bounded_page() {
        let root = temp_root("read-default-page");
        let file = root.join("big.txt");
        let body: String = (1..=500).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&file, body).unwrap();

        let page = read_page(&file, &serde_json::json!({}), "big.txt").unwrap();
        assert_eq!(page.start_line, 1);
        assert_eq!(page.end_line, DEFAULT_READ_LINES);
        assert_eq!(page.total_lines, 500);
        assert!(page.has_more);
        assert_eq!(page.next_offset, Some(DEFAULT_READ_LINES + 1));
        assert!(!page.content.contains("line 401\t"));
    }

    #[test]
    fn read_stops_at_the_default_byte_budget() {
        let root = temp_root("read-byte-budget");
        let file = root.join("wide.txt");
        let line = "x".repeat(1_000);
        let body: String = (1..=60).map(|_| format!("{line}\n")).collect();
        std::fs::write(&file, body).unwrap();

        let page = read_page(&file, &serde_json::json!({}), "wide.txt").unwrap();
        assert!(page.content.len() <= DEFAULT_READ_BYTES);
        assert!(page.has_more);
        assert_eq!(page.end_line, page.next_offset.unwrap() - 1);
        assert!(page.end_line < page.total_lines);
    }

    #[test]
    fn read_pages_from_next_offset() {
        let root = temp_root("read-next-offset");
        let file = root.join("big.txt");
        let body: String = (1..=500).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&file, body).unwrap();

        let page = read_page(&file, &serde_json::json!({ "offset": 401 }), "big.txt").unwrap();
        assert_eq!(page.start_line, 401);
        assert_eq!(page.end_line, 500);
        assert!(!page.has_more);
        assert_eq!(page.next_offset, None);
    }

    #[test]
    fn read_of_an_oversized_file_returns_head_and_tail() {
        let root = temp_root("read-head-tail");
        let file = root.join("huge.txt");
        let body = format!("head-marker\n{}\ntail-marker\n", "a".repeat(MAX_READ_BYTES));
        std::fs::write(&file, body).unwrap();

        let page = read_page(&file, &serde_json::json!({}), "huge.txt").unwrap();
        assert!(page.truncated);
        assert_eq!(page.next_offset, None);
        assert!(page.content.contains("head-marker"));
        assert!(page.content.contains("tail-marker"));
        assert!(page.content.contains("truncated"));
    }

    #[test]
    fn read_of_an_empty_file_reports_no_lines() {
        let root = temp_root("read-empty");
        let file = root.join("empty.txt");
        std::fs::write(&file, "").unwrap();

        let page = read_page(&file, &serde_json::json!({}), "empty.txt").unwrap();
        assert_eq!(page.content, "");
        assert_eq!(page.total_lines, 0);
        assert_eq!(page.start_line, 0);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_replaces_a_symlink_instead_of_writing_through_it() {
        let root = temp_root("atomic-symlink");
        let outside =
            std::env::temp_dir().join(format!("mixd-files-atomic-outside-{}", std::process::id()));
        std::fs::write(&outside, b"original").unwrap();
        let target = root.join("target.txt");
        std::os::unix::fs::symlink(&outside, &target).unwrap();

        atomic_write_bytes(&target, b"replacement").unwrap();

        assert_eq!(std::fs::read(&outside).unwrap(), b"original");
        assert!(!std::fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"replacement");
        let _ = std::fs::remove_file(&outside);
    }
}
