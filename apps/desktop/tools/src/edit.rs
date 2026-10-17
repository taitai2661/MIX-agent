//! Targeted file editing.
//!
//! `files.write` re-emits an entire file, which is token-expensive and
//! error-prone for real code changes. This tool applies the smallest possible
//! change instead: it anchors on an exact string, refuses to guess when that
//! anchor is ambiguous, and writes atomically.
//!
//! The rules deliberately mirror what a careful human editor does:
//!
//! - the anchor must exist, otherwise the edit is rejected rather than guessed;
//! - a non-`replace_all` anchor must be unique, otherwise the caller is asked
//!   to widen the context;
//! - trailing-whitespace and CRLF differences are tolerated, because models
//!   reproduce them inconsistently, but nothing else is.

use std::path::Path;

use async_trait::async_trait;

use super::files::FilesTool;
use super::{Tool, ToolContext, ToolResult};

/// Refuse to load files larger than this into memory for an edit.
pub const MAX_EDIT_BYTES: u64 = 4 * 1024 * 1024;

/// How many context lines to keep around a change for the returned preview.
const PREVIEW_CONTEXT_LINES: usize = 3;

#[derive(Default)]
pub struct EditTool;

/// Outcome of a successful edit, returned to the model so it can confirm.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EditOutcome {
    pub path: String,
    pub action: String,
    /// Number of anchor occurrences replaced (1 unless `replace_all`).
    pub replacements: usize,
    pub lines_before: usize,
    pub lines_after: usize,
    pub created: bool,
    /// Unified-diff style preview of the changed region.
    pub preview: String,
}

/// Outcome of a successful `multi_edit` batch.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MultiEditOutcome {
    /// Per-file outcomes, in the order the entries were given.
    pub edits: Vec<EditOutcome>,
}

impl EditTool {
    /// Replace every occurrence of `old` with `new`.
    ///
    /// When `replace_all` is false the anchor must occur exactly once. Matching
    /// first tries the literal text, then a whitespace-tolerant comparison so
    /// that trailing spaces or CRLF endings do not defeat the edit.
    pub fn replace(
        content: &str,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> Result<(String, usize), EditError> {
        if old.is_empty() {
            return Err(EditError::EmptyAnchor);
        }
        if old == new {
            return Err(EditError::NoOp);
        }

        let occurrences = content.matches(old).count();
        if occurrences == 0 {
            // Fall back to a whitespace-tolerant comparison before giving up.
            let loose = loose_occurrences(content, old);
            if loose == 0 {
                return Err(EditError::AnchorNotFound);
            }
            if !replace_all && loose > 1 {
                return Err(EditError::Ambiguous(loose));
            }
            return Ok((loose_replace(content, old, new, replace_all), loose));
        }
        if !replace_all && occurrences > 1 {
            return Err(EditError::Ambiguous(occurrences));
        }
        let updated = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };
        Ok((updated, occurrences))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("old_string must not be empty; include enough context to be unique")]
    EmptyAnchor,
    #[error("old_string and new_string are identical; nothing to do")]
    NoOp,
    #[error(
        "old_string was not found in the file. Re-read the file and copy the anchor \
         exactly, including indentation."
    )]
    AnchorNotFound,
    #[error(
        "old_string occurs {0} times. Add surrounding lines to make it unique, or pass \
         replace_all: true to change every occurrence."
    )]
    Ambiguous(usize),
    #[error("new_string would exceed the {MAX_EDIT_BYTES} byte edit limit")]
    TooLarge,
}

/// Count matches after normalising trailing whitespace per line.
fn loose_occurrences(content: &str, old: &str) -> usize {
    let needle: Vec<&str> = old.lines().map(trim_end).collect();
    content
        .lines()
        .collect::<Vec<_>>()
        .windows(needle.len())
        .filter(|window| {
            window
                .iter()
                .map(|line| trim_end(line))
                .eq(needle.iter().copied())
        })
        .count()
}

fn loose_replace(content: &str, old: &str, new: &str, replace_all: bool) -> String {
    let needle: Vec<&str> = old.lines().map(trim_end).collect();
    let replacement: Vec<&str> = new.lines().collect();
    let source: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(source.len());
    let mut i = 0usize;
    while i < source.len() {
        let matches_here = i + needle.len() <= source.len()
            && source[i..i + needle.len()]
                .iter()
                .map(|line| trim_end(line))
                .eq(needle.iter().copied());
        if matches_here {
            out.extend(replacement.iter().map(|line| line.to_string()));
            i += needle.len();
            if !replace_all {
                // Everything after the anchor still belongs to the file. This
                // used to `break` without copying it, so a whitespace-tolerant
                // match silently deleted the rest of the file.
                out.extend(source[i..].iter().map(|line| line.to_string()));
                break;
            }
        } else {
            out.push(source[i].to_string());
            i += 1;
        }
    }
    let trailing_newline = content.ends_with('\n');
    let mut joined = out.join("\n");
    if trailing_newline && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

fn trim_end(line: &str) -> &str {
    line.trim_end_matches(['\r', ' ', '\t'])
}

/// Render a minimal `@@`-style preview around the first changed line so the
/// model can confirm the edit landed where it intended.
fn preview_of(before: &str, after: &str) -> String {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let anchor = first_difference(&before_lines, &after_lines).unwrap_or(0);
    let start = anchor.saturating_sub(PREVIEW_CONTEXT_LINES);
    let end = (anchor + PREVIEW_CONTEXT_LINES + 1).min(after_lines.len());
    let mut out = format!("@@ line {} @@\n", anchor + 1);
    for (offset, line) in after_lines[start..end].iter().enumerate() {
        let number = start + offset + 1;
        let existed = before_lines
            .get(number - 1)
            .map(|b| b.trim_end() == line.trim_end())
            .unwrap_or(false);
        out.push_str(&format!(
            "{}{} {}\n",
            if existed { ' ' } else { '+' },
            number,
            line
        ));
    }
    out
}

fn first_difference(before: &[&str], after: &[&str]) -> Option<usize> {
    if before == after {
        return None;
    }
    for (idx, (a, b)) in before.iter().zip(after.iter()).enumerate() {
        if a.trim_end() != b.trim_end() {
            return Some(idx);
        }
    }
    Some(before.len().min(after.len()))
}

/// Write `content` to `path` through a temp file so a crash cannot truncate it.
fn atomic_write(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = super::files::temp_sibling(path);
    if let Err(err) = std::fs::write(&tmp, content) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

#[async_trait]
impl Tool for EditTool {
    fn id(&self) -> &str {
        "edit"
    }
    fn name(&self) -> &str {
        "Edit"
    }
    fn description(&self) -> &str {
        "Make a targeted change to a file by replacing an exact string, creating a new \
         file, or appending to one. Prefer this over rewriting a whole file: quote the \
         smallest old_string that is unique, and include surrounding lines if it appears \
         more than once."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["replace", "create", "append"],
                    "default": "replace",
                    "description": "replace: swap old_string for new_string. create: write a \
                                    new file (fails if it already exists). append: add \
                                    new_string to the end of an existing file."
                },
                "path": { "type": "string", "description": "Path relative to the workspace root." },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to replace, including indentation. Required for replace."
                },
                "new_string": { "type": "string", "description": "Replacement text." },
                "replace_all": {
                    "type": "boolean",
                    "default": false,
                    "description": "Replace every occurrence instead of requiring a unique anchor."
                },
                "edits": {
                    "type": "array",
                    "description": "Batch of edits to apply atomically. When present, the \
                                     single-edit fields above are ignored. Every entry is \
                                     applied in order; if any entry fails, no file is written.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" },
                            "replace_all": { "type": "boolean", "default": false }
                        },
                        "required": ["path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["path", "new_string"]
        })
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        // Read, anchor matching and the atomic write are all synchronous; the
        // `write` + `rename` pair at the end must stay together on one thread
        // so the temp file is never observed half-written, which is exactly
        // what `spawn_blocking` gives us (a dedicated thread per task).
        //
        // Action and path are copied out before the move so a failed edit can
        // be logged with the file it touched.
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("replace")
            .to_string();
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let root_path = ctx.root_path;
        let additional = ctx.additional_paths;
        match tokio::task::spawn_blocking(move || {
            EditTool::invoke_blocking(root_path, additional, args)
        })
        .await
        {
            Ok(outcome) => {
                if let Err(err) = &outcome {
                    tracing::warn!(action, path, %err, "edit tool failed");
                }
                outcome
            }
            Err(join_err) => {
                tracing::error!(action, path, %join_err, "edit tool worker panicked");
                Err(anyhow::anyhow!("edit worker failed: {join_err}"))
            }
        }
    }
}

impl EditTool {
    fn invoke_blocking(
        root_path: Option<String>,
        additional: Vec<String>,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        // A batch of edits takes the atomic multi-file path; the single-edit
        // fields are ignored when `edits` is present.
        if let Some(batch) = args.get("edits").and_then(|v| v.as_array()) {
            return EditTool::invoke_multi_blocking(root_path, additional, batch.clone());
        }

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("replace")
            .to_string();
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("path required"))?
            .to_string();
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("new_string required"))?
            .to_string();
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let workspace_root = FilesTool::resolve_context_root(root_path.as_deref())?;
        let (resolved, writable) =
            FilesTool::resolve_target_in(&workspace_root, &additional, &path)?;
        if !writable {
            return Ok(failure(&format!("追加フォルダは読み取り専用です: {path}")));
        }
        let before = read_existing(&resolved, &action)?;

        let (after, replacements, created) = match action.as_str() {
            "create" => {
                if resolved.exists() {
                    return Ok(failure(&format!(
                        "{path} already exists; use action \"replace\" to edit it"
                    )));
                }
                (new_string.clone(), 1, true)
            }
            "append" => {
                let mut joined = before.clone();
                if !joined.is_empty() && !joined.ends_with('\n') {
                    joined.push('\n');
                }
                joined.push_str(&new_string);
                (joined, 1, false)
            }
            "replace" => {
                let old_string = args
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("old_string required for replace"))?;
                let (updated, count) =
                    EditTool::replace(&before, old_string, &new_string, replace_all)?;
                (updated, count, false)
            }
            other => return Ok(failure(&format!("unknown action: {other}"))),
        };

        if after.len() as u64 > MAX_EDIT_BYTES {
            return Ok(failure(&format!(
                "result would be {} bytes, over the {MAX_EDIT_BYTES} byte limit",
                after.len()
            )));
        }
        if before.is_empty() && after.is_empty() {
            return Ok(failure("new_string is empty; nothing would be written"));
        }

        atomic_write(&resolved, &after)?;
        let outcome = EditOutcome {
            path: path.clone(),
            action,
            replacements,
            lines_before: before.lines().count(),
            lines_after: after.lines().count(),
            created,
            preview: preview_of(&before, &after),
        };
        Ok(ToolResult {
            ok: true,
            content: serde_json::to_value(&outcome).unwrap_or_else(|_| serde_json::json!({})),
            error: None,
        })
    }

    /// Apply a batch of edits atomically: every entry is applied in order, and
    /// if any entry fails, no file is written.
    ///
    /// The whole batch is held in memory before the first `atomic_write`, so a
    /// failure on the last entry leaves the earlier files untouched. This is
    /// the same all-or-nothing guarantee the single-edit path gives, extended
    /// to multiple files.
    fn invoke_multi_blocking(
        root_path: Option<String>,
        additional: Vec<String>,
        batch: Vec<serde_json::Value>,
    ) -> anyhow::Result<ToolResult> {
        if batch.is_empty() {
            return Ok(failure("edits must not be empty"));
        }
        // A batch that repeats a path would make the result depend on the order
        // of application in a way that is hard to reason about, so reject it
        // outright rather than silently applying edits in sequence.
        let mut seen_paths = std::collections::HashSet::new();
        for entry in &batch {
            let entry_path = entry
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("every edits entry requires a path"))?;
            if !seen_paths.insert(entry_path.to_string()) {
                return Ok(failure(&format!(
                    "edits contains duplicate path: {entry_path}"
                )));
            }
        }

        let workspace_root = FilesTool::resolve_context_root(root_path.as_deref())?;
        // Read every file up front so a missing file fails before any write.
        let mut before_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut resolved_map: std::collections::HashMap<String, std::path::PathBuf> =
            std::collections::HashMap::new();
        for entry in &batch {
            let entry_path = entry.get("path").and_then(|v| v.as_str()).unwrap();
            let (resolved, writable) =
                FilesTool::resolve_target_in(&workspace_root, &additional, entry_path)?;
            if !writable {
                return Ok(failure(&format!(
                    "追加フォルダは読み取り専用です: {entry_path}"
                )));
            }
            let before = read_existing(&resolved, "replace")?;
            before_map.insert(entry_path.to_string(), before);
            resolved_map.insert(entry_path.to_string(), resolved);
        }

        // Apply every entry in order, accumulating the after-content. A failure
        // here means nothing has been written yet, so the caller gets an error
        // and the files are untouched.
        let mut outcomes: Vec<EditOutcome> = Vec::with_capacity(batch.len());
        let mut after_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for entry in &batch {
            let entry_path = entry.get("path").and_then(|v| v.as_str()).unwrap();
            let old_string = entry
                .get("old_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("every edits entry requires old_string"))?;
            let new_string = entry
                .get("new_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("every edits entry requires new_string"))?;
            let replace_all = entry
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let before = before_map.get(entry_path).unwrap();
            let (after, replacements) =
                EditTool::replace(before, old_string, new_string, replace_all)?;

            if after.len() as u64 > MAX_EDIT_BYTES {
                return Ok(failure(&format!(
                    "result would be {} bytes, over the {MAX_EDIT_BYTES} byte limit",
                    after.len()
                )));
            }
            if before.is_empty() && after.is_empty() {
                return Ok(failure("new_string is empty; nothing would be written"));
            }

            outcomes.push(EditOutcome {
                path: entry_path.to_string(),
                action: "replace".to_string(),
                replacements,
                lines_before: before.lines().count(),
                lines_after: after.lines().count(),
                created: false,
                preview: preview_of(before, &after),
            });
            after_map.insert(entry_path.to_string(), after);
        }

        // All entries applied cleanly: write every file. If writing any file fails,
        // rollback all previously written files to their `before` contents so
        // multi_edit remains all-or-nothing (atomic) across the batch.
        let mut written_paths: Vec<(&str, &std::path::PathBuf)> = Vec::new();
        for (entry_path, after) in &after_map {
            let resolved = resolved_map.get(entry_path).unwrap();
            if let Err(err) = atomic_write(resolved, after) {
                // Rollback previously written files in reverse order
                for (prev_path, prev_resolved) in written_paths.iter().rev() {
                    if let Some(orig) = before_map.get(*prev_path) {
                        let _ = atomic_write(prev_resolved, orig);
                    }
                }
                return Err(err);
            }
            written_paths.push((entry_path.as_str(), resolved));
        }

        let outcome = MultiEditOutcome { edits: outcomes };
        Ok(ToolResult {
            ok: true,
            content: serde_json::to_value(&outcome).unwrap_or_else(|_| serde_json::json!({})),
            error: None,
        })
    }
}

fn read_existing(path: &Path, action: &str) -> anyhow::Result<String> {
    if action == "create" {
        return Ok(String::new());
    }
    let meta = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
    if meta.is_dir() {
        anyhow::bail!("{} is a directory", path.display());
    }
    if meta.len() > MAX_EDIT_BYTES {
        anyhow::bail!(
            "{} is {} bytes, over the {MAX_EDIT_BYTES} byte edit limit",
            path.display(),
            meta.len()
        );
    }
    Ok(std::fs::read_to_string(path)?)
}

/// Edit failures are reported to the model as `ok: false` rather than as a
/// transport error, so the agent can read the message and retry with a wider
/// anchor instead of aborting the run.
fn failure(message: &str) -> ToolResult {
    ToolResult {
        ok: false,
        content: serde_json::json!({}),
        error: Some(message.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_a_unique_anchor() {
        let (out, count) = EditTool::replace("a\nb\nc\n", "b", "B", false).expect("replace");
        assert_eq!(out, "a\nB\nc\n");
        assert_eq!(count, 1);
    }

    #[test]
    fn rejects_an_ambiguous_anchor() {
        let err = EditTool::replace("x\nx\n", "x", "y", false).expect_err("ambiguous");
        assert!(matches!(err, EditError::Ambiguous(2)));
    }

    #[test]
    fn replace_all_rewrites_every_occurrence() {
        let (out, count) = EditTool::replace("x x x", "x", "y", true).expect("replace all");
        assert_eq!(out, "y y y");
        assert_eq!(count, 3);
    }

    #[test]
    fn rejects_a_missing_anchor() {
        let err = EditTool::replace("hello", "goodbye", "hi", false).expect_err("missing");
        assert!(matches!(err, EditError::AnchorNotFound));
    }

    #[test]
    fn rejects_an_empty_anchor() {
        assert!(matches!(
            EditTool::replace("hello", "", "hi", false).expect_err("empty"),
            EditError::EmptyAnchor
        ));
    }

    #[test]
    fn rejects_a_no_op_edit() {
        assert!(matches!(
            EditTool::replace("hello", "hello", "hello", false).expect_err("no-op"),
            EditError::NoOp
        ));
    }

    #[test]
    fn tolerates_trailing_whitespace_drift() {
        // The model reproduced the line with a trailing space and CRLF.
        let (out, count) = EditTool::replace(
            "fn main() {   \r\n    run();\r\n}\r\n",
            "    run();",
            "    run();\n    done();",
            false,
        )
        .expect("loose replace");
        assert_eq!(count, 1);
        assert!(out.contains("    done();"));
        assert!(!out.contains("run();   \r"));
    }

    #[test]
    fn a_whitespace_tolerant_match_keeps_the_rest_of_the_file() {
        // The anchor only matches after trimming, which is exactly the loose
        // path; everything after it has to survive.
        let (out, count) = EditTool::replace("head\nmiddle\nfoot\n", "middle   ", "changed", false)
            .expect("loose replace");
        assert_eq!(count, 1);
        assert_eq!(out, "head\nchanged\nfoot\n");
    }

    #[test]
    fn reports_ambiguity_after_whitespace_tolerance() {
        let err = EditTool::replace(
            "let a = 1;  \nlet a = 1;\n",
            "let a = 1;",
            "let a = 2;",
            false,
        )
        .expect_err("ambiguous");
        assert!(matches!(err, EditError::Ambiguous(2)));
    }

    #[test]
    fn preview_marks_changed_lines() {
        let preview = preview_of("a\nb\nc\n", "a\nB\nc\n");
        assert!(preview.contains("@@ line 2 @@"));
        assert!(preview.contains("+2 B"));
        assert!(preview.contains(" 1 a"));
    }

    #[test]
    fn atomic_write_replaces_content() {
        let dir = std::env::temp_dir().join(format!("mixd-edit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.txt");
        atomic_write(&path, "one\n").unwrap();
        atomic_write(&path, "two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        // The temp file must not be left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("mix-agent-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multi_edit_atomicity_rolls_back_or_aborts_before_write() {
        let dir = std::env::temp_dir().join(format!("mixd-multi-edit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file1 = dir.join("file1.txt");
        let file2 = dir.join("file2.txt");
        std::fs::write(&file1, "first file original").unwrap();
        std::fs::write(&file2, "second file original").unwrap();

        let batch = vec![
            serde_json::json!({
                "path": "file1.txt",
                "old_string": "first file original",
                "new_string": "first file updated"
            }),
            serde_json::json!({
                "path": "file2.txt",
                "old_string": "non-existent anchor",
                "new_string": "second file updated"
            }),
        ];

        // Should fail on file2 anchor match and leave file1 untouched
        let res = EditTool::invoke_multi_blocking(Some(dir.display().to_string()), Vec::new(), batch);
        assert!(res.is_err() || res.as_ref().is_ok_and(|r| !r.ok));
        assert_eq!(std::fs::read_to_string(&file1).unwrap(), "first file original");
        assert_eq!(std::fs::read_to_string(&file2).unwrap(), "second file original");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
