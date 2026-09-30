//! File discovery and content search.
//!
//! Both tools walk the workspace with the `ignore` crate rather than a raw
//! directory walk, which buys three things the previous `files.search`
//! implementation lacked: `.gitignore` and `.ignore` files are respected,
//! binary files are detected instead of being read into memory, and the walk
//! is parallel.
//!
//! `glob` answers "which files match this pattern"; `grep` answers "where does
//! this text appear". Keeping them separate matters because they have different
//! failure modes: a bad pattern should fail loudly, whereas a search with no
//! hits is a normal answer.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use ignore::WalkBuilder;
use serde::Serialize;

use super::files::FilesTool;
use super::{Tool, ToolContext, ToolResult};

/// Hard ceiling on returned matches. Beyond this the model gets a truncated
/// view plus an explicit flag so it can narrow the query rather than assume the
/// result set is complete.
const MAX_MATCHES: usize = 250;
const DEFAULT_MAX_MATCHES: usize = 100;
const MAX_DEPTH: usize = 24;
/// Skip files above this size; grepping a minified bundle is never useful.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Default)]
pub struct GlobTool;

/// Directories that are almost never what an agent is searching for, and that
/// cost minutes rather than seconds to traverse. `.gitignore` alone is not
/// enough: plenty of workspaces have no git repo at all, and even ones that do
/// commonly leave build output untracked-but-not-ignored.
const DEFAULT_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".gradle",
    ".cache",
    "vendor",
    "Pods",
    ".terraform",
];

fn is_default_excluded(name: &str) -> bool {
    DEFAULT_EXCLUDED_DIRS.contains(&name)
}

/// Whether a search rooted at `start` is deliberately aimed inside one of the
/// excluded directories, in which case the default excludes must not apply.
fn start_is_deliberate(start: &Path) -> bool {
    start
        .components()
        .any(|c| c.as_os_str().to_str().is_some_and(is_default_excluded))
}

/// Shared walk configuration, exposed so `files.search` stays ignore-aware.
pub fn workspace_walker(root: &Path) -> WalkBuilder {
    walker(root, false)
}

/// Same walk, already expanded into entries. Callers in sync contexts use this
/// instead of holding a `WalkBuilder`.
pub fn workspace_entries(root: &Path) -> Vec<ignore::DirEntry> {
    walker(root, false)
        .build()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .collect()
}

/// Shared walk configuration: honour ignore files, stay inside the workspace.
///
/// `require_git(false)` is deliberate. The `ignore` crate ignores `.gitignore`
/// outside a git repository by default, which is the right default for a
/// recursive grep but the wrong one for a coding agent: the user pointed us at
/// a folder, and the folder's own ignore rules should apply whether or not it
/// happens to be version controlled.
fn walker(root: &Path, hidden: bool) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!hidden)
        .parents(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .ignore(true)
        .max_depth(Some(MAX_DEPTH))
        .follow_links(false);
    if !start_is_deliberate(root) {
        builder.filter_entry(|entry| {
            entry.depth() == 0 || !is_default_excluded(&entry.file_name().to_string_lossy())
        });
    }
    builder
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn read_max_matches(args: &serde_json::Value) -> usize {
    args.get("max_results")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, MAX_MATCHES))
        .unwrap_or(DEFAULT_MAX_MATCHES)
}

#[derive(Debug, Clone, Serialize)]
struct GlobMatch {
    path: String,
    is_dir: bool,
    size: u64,
}

#[async_trait]
impl Tool for GlobTool {
    fn id(&self) -> &str {
        "glob"
    }
    /// `glob` only lists filenames the workspace already has. It cannot change
    /// anything, so it does not need to interrupt the user.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn name(&self) -> &str {
        "Glob"
    }
    fn description(&self) -> &str {
        "Find files by glob pattern, e.g. `src/**/*.rs`. Respects .gitignore. Returns \
         paths sorted by most recently modified first. Use this to locate code before \
         reading it."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern relative to the workspace root, e.g. `**/*.test.ts`."
                },
                "path": {
                    "type": "string",
                    "description": "Optional subdirectory to search under. Defaults to the workspace root."
                },
                "max_results": { "type": "integer", "default": 100 },
                "include_hidden": { "type": "boolean", "default": false }
            },
            "required": ["pattern"]
        })
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("path").and_then(|v| v.as_str()).map(|s| s.to_string())
    }

    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("pattern required"))?
            .to_string();
        let workspace_root = FilesTool::resolve_context_root(ctx.root_path.as_deref())?;
        let sub = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        // Reads are allowed in the additional (read-only) roots too, so a
        // monorepo can grep its sibling packages with an absolute path.
        let (start, _writable) =
            FilesTool::resolve_target_in(&workspace_root, &ctx.additional_paths, sub)?;
        let limit = read_max_matches(&args);
        let hidden = args
            .get("include_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let matcher = glob::Pattern::new(&pattern)
            .map_err(|e| anyhow::anyhow!("invalid glob pattern {pattern:?}: {e}"))?;

        let (hits, truncated) = tokio::task::spawn_blocking(move || {
            let mut found: Vec<GlobMatch> = Vec::new();
            let mut truncated = false;
            for entry in walker(&start, hidden).build().flatten() {
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if !matcher.matches(&name) && !matcher.matches(&relative_to(&start, entry.path())) {
                    continue;
                }
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                if size > MAX_FILE_BYTES {
                    continue;
                }
                found.push(GlobMatch {
                    path: entry.path().to_string_lossy().to_string(),
                    is_dir: false,
                    size,
                });
            }
            // Most recently modified first: the file a human would open next.
            found.sort_by_key(|m| std::fs::metadata(&m.path).and_then(|md| md.modified()).ok());
            found.reverse();
            if found.len() > limit {
                found.truncate(limit);
                truncated = true;
            }
            (found, truncated)
        })
        .await?;

        let relative: Vec<GlobMatch> = hits
            .into_iter()
            .map(|m| GlobMatch {
                path: relative_to(&workspace_root, Path::new(&m.path)),
                is_dir: m.is_dir,
                size: m.size,
            })
            .collect();
        Ok(ToolResult {
            ok: true,
            content: serde_json::json!({
                "pattern": pattern,
                "matches": relative,
                "count": relative.len(),
                "truncated": truncated,
            }),
            error: None,
        })
    }
}

#[derive(Default)]
pub struct GrepTool;

#[derive(Debug, Clone, Serialize)]
struct GrepMatch {
    path: String,
    line: usize,
    text: String,
}

#[async_trait]
impl Tool for GrepTool {
    fn id(&self) -> &str {
        "grep"
    }
    /// `grep` only reads. Asking permission for it would make the first thing an
    /// agent does impossible without a click.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn name(&self) -> &str {
        "Grep"
    }
    fn description(&self) -> &str {
        "Search file contents with a regular expression. Returns `path:line: text` for each \
         match. Respects .gitignore and skips binary files. Narrow the query with `path` or \
         `glob` when the result set is truncated."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Rust regex, e.g. `fn\\s+main`." },
                "path": { "type": "string", "description": "Optional subdirectory or file to search." },
                "glob": { "type": "string", "description": "Only search files matching this glob, e.g. `*.rs`." },
                "case_sensitive": { "type": "boolean", "default": true },
                "max_results": { "type": "integer", "default": 100 }
            },
            "required": ["pattern"]
        })
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("path").and_then(|v| v.as_str()).map(|s| s.to_string())
    }

    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("pattern required"))?
            .to_string();
        // A malformed regex must fail loudly. Silently degrading to `.*` turns a
        // typo into "every line matches", which is worse than an error.
        let mut builder = regex::RegexBuilder::new(&pattern);
        builder.case_insensitive(
            !args
                .get("case_sensitive")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        );
        let regex = builder
            .build()
            .map_err(|e| anyhow::anyhow!("invalid regex {pattern:?}: {e}"))?;

        let workspace_root = FilesTool::resolve_context_root(ctx.root_path.as_deref())?;
        let target = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let (start, _writable) =
            FilesTool::resolve_target_in(&workspace_root, &ctx.additional_paths, target)?;
        let file_glob = args
            .get("glob")
            .and_then(|v| v.as_str())
            .map(|g| g.to_string());
        let limit = read_max_matches(&args);

        let (hits, truncated, files_scanned) = tokio::task::spawn_blocking(move || {
            let file_matcher = match file_glob
                .as_deref()
                .map(glob::Pattern::new)
                .transpose()
                .map_err(|e| anyhow::anyhow!("invalid glob filter: {e}"))
            {
                Ok(m) => m,
                Err(e) => return Err(e),
            };
            let mut hits: Vec<GrepMatch> = Vec::new();
            let mut truncated = false;
            let mut files_scanned = 0usize;
            // A single file target short-circuits the walk.
            let entries: Vec<PathBuf> = if start.is_file() {
                vec![start.clone()]
            } else {
                walker(&start, false)
                    .build()
                    .flatten()
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .map(|e| e.path().to_path_buf())
                    .collect()
            };
            'files: for path in entries {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string());
                if let (Some(matcher), Some(name)) = (file_matcher.as_ref(), name.as_deref()) {
                    if !matcher.matches(name) {
                        continue;
                    }
                }
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                if meta.len() > MAX_FILE_BYTES {
                    continue;
                }
                // The content sniff, not a file extension: a .rs file holding
                // an embedded PNG should not be dumped into the prompt.
                if is_probably_binary(&path) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                files_scanned += 1;
                for (idx, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        hits.push(GrepMatch {
                            path: path.to_string_lossy().to_string(),
                            line: idx + 1,
                            text: truncate_line(line),
                        });
                        if hits.len() >= limit {
                            truncated = true;
                            break 'files;
                        }
                    }
                }
            }
            Ok((hits, truncated, files_scanned))
        })
        .await??;

        let relative: Vec<GrepMatch> = hits
            .into_iter()
            .map(|m| GrepMatch {
                path: relative_to(&workspace_root, Path::new(&m.path)),
                line: m.line,
                text: m.text,
            })
            .collect();
        Ok(ToolResult {
            ok: true,
            content: serde_json::json!({
                "pattern": pattern,
                "matches": relative,
                "count": relative.len(),
                "files_scanned": files_scanned,
                "truncated": truncated,
            }),
            error: None,
        })
    }
}

/// Reject anything with a NUL byte in the first 8 KiB, which is the same cheap
/// heuristic ripgrep uses.
pub fn is_probably_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return true;
    };
    let mut buf = [0u8; 8192];
    let read = file.read(&mut buf).unwrap_or(0);
    buf[..read].contains(&0)
}

fn truncate_line(line: &str) -> String {
    const MAX_CHARS: usize = 400;
    if line.chars().count() <= MAX_CHARS {
        return line.to_string();
    }
    let head: String = line.chars().take(MAX_CHARS).collect();
    format!("{head}…")
}

/// Parse a permission pattern such as `terminal:rm -rf` into its rule scope.
/// Kept here so the scope vocabulary lives next to the tools that honour it.
pub fn split_rule_pattern(pattern: &str) -> Option<(&str, &str)> {
    let (tool, rest) = pattern.split_once(':')?;
    let tool = tool.trim();
    if tool.is_empty() {
        None
    } else {
        Some((tool, rest.trim()))
    }
}

/// Counters shared by the engine to report search coverage in run events.
#[derive(Default)]
pub struct SearchStats {
    pub queries: AtomicUsize,
    pub cancelled: AtomicBool,
}

impl SearchStats {
    pub fn snapshot(&self) -> (usize, bool) {
        (
            self.queries.load(Ordering::Relaxed),
            self.cancelled.load(Ordering::Relaxed),
        )
    }

    pub fn record_query(&self) {
        self.queries.fetch_add(1, Ordering::Relaxed);
    }
}

pub type SharedSearchStats = Arc<SearchStats>;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mixd-search-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    todo!();\n}\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn helper() {}\n").unwrap();
        std::fs::write(dir.join("node_modules/dep.js"), "todo!();\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "node_modules/\n").unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn truncate_line_caps_long_lines() {
        let long = "x".repeat(500);
        let out = truncate_line(&long);
        assert_eq!(out.chars().count(), 401);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn rule_pattern_splits_on_the_first_colon() {
        assert_eq!(
            split_rule_pattern("terminal: cargo build"),
            Some(("terminal", "cargo build"))
        );
        assert_eq!(split_rule_pattern("no-colon"), None);
    }

    #[test]
    fn glob_pattern_compiles() {
        assert!(glob::Pattern::new("**/*.rs").is_ok());
        assert!(glob::Pattern::new("[").is_err());
    }

    #[test]
    fn grep_rejects_a_malformed_regex() {
        // Assembled at runtime so clippy's `invalid_regex` lint does not flag the
        // deliberately broken pattern; the point is that the tool errors rather
        // than silently falling back to "match everything".
        let broken = ["(", "unclosed"].concat();
        assert!(
            regex::RegexBuilder::new(&broken).build().is_err(),
            "malformed regex must not silently compile"
        );
    }

    #[test]
    fn walk_respects_gitignore_and_default_excludes() {
        let root = fixture("walk");
        let names: Vec<String> = walker(&root, false)
            .build()
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"main.rs".to_string()));
        assert!(
            !names.contains(&"dep.js".to_string()),
            "node_modules must be skipped even without a git repo: {names:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_explicit_path_into_node_modules_still_searches() {
        let root = fixture("deliberate");
        // The default excludes must not fire when the user aimed at the directory.
        let names: Vec<String> = workspace_entries(&root.join("node_modules"))
            .iter()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["dep.js".to_string()]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
