//! Symbol outline and references exploration tool.
//!
//! Provides structural code discovery without needing a full LSP:
//!  - `outline`: Extracts top-level and inner definitions (functions, classes, structs, traits, types) in a file.
//!  - `references`: Finds usages of a specific symbol across the workspace respecting `.gitignore`.

use std::path::Path;

use async_trait::async_trait;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use super::files::FilesTool;
use super::search::MAX_FILE_BYTES;
use super::{Tool, ToolContext, ToolResult};

#[derive(Default)]
pub struct SymbolsTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineItem {
    pub line: usize,
    pub kind: String,
    pub name: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceItem {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[async_trait]
impl Tool for SymbolsTool {
    fn id(&self) -> &str {
        "symbols"
    }

    fn name(&self) -> &str {
        "Symbols"
    }

    fn description(&self) -> &str {
        "Inspect code structure and symbol usages. Use action: \"outline\" with a path to list \
         functions, classes, structs, traits, and types with their line numbers. Use action: \
         \"references\" with a symbol name to find everywhere it is used in the workspace. Use \
         action: \"map\" for a ranked, repo-wide overview of files and their top-level symbols \
         before exploring individual files."
    }

    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["outline", "references", "map"],
                    "default": "outline",
                    "description": "outline: list symbol definitions in a file. references: search for symbol occurrences across workspace. map: rank files and summarize their symbols."
                },
                "path": {
                    "type": "string",
                    "description": "Path relative to the workspace root (required for outline)."
                },
                "symbol": {
                    "type": "string",
                    "description": "Symbol name to search for (required for references)."
                },
                "max_results": {
                    "type": "integer",
                    "default": 100,
                    "description": "Maximum number of items to return."
                }
            }
        })
    }

    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("outline")
            .to_string();
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;

        let root_path = ctx.root_path;
        let additional = ctx.additional_paths;

        match tokio::task::spawn_blocking(move || {
            let workspace_root = FilesTool::resolve_context_root(root_path.as_deref())?;
            match action.as_str() {
                "outline" => {
                    let path = args
                        .get("path")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("path is required for action: outline"))?;
                    let (resolved, _) =
                        FilesTool::resolve_target_in(&workspace_root, &additional, path)?;
                    let items = extract_outline(&resolved)?;
                    let total = items.len();
                    let limited: Vec<OutlineItem> = items.into_iter().take(max_results).collect();
                    Ok(ToolResult {
                        ok: true,
                        content: serde_json::json!({
                            "path": path,
                            "symbols": limited,
                            "total": total,
                            "truncated": total > max_results,
                        }),
                        error: None,
                    })
                }
                "references" => {
                    let symbol = args.get("symbol").and_then(|v| v.as_str()).ok_or_else(|| {
                        anyhow::anyhow!("symbol is required for action: references")
                    })?;
                    if symbol.trim().is_empty() {
                        return Ok(ToolResult {
                            ok: false,
                            content: serde_json::json!({}),
                            error: Some("symbol cannot be empty".to_string()),
                        });
                    }
                    let refs = find_references(&workspace_root, symbol, max_results)?;
                    let total = refs.len();
                    Ok(ToolResult {
                        ok: true,
                        content: serde_json::json!({
                            "symbol": symbol,
                            "references": refs,
                            "total": total,
                            "truncated": total >= max_results,
                        }),
                        error: None,
                    })
                }
                "map" => {
                    let map = build_repo_map(&workspace_root, max_results)?;
                    Ok(ToolResult {
                        ok: true,
                        content: serde_json::json!({
                            "files": map.files,
                            "total_files": map.total_files,
                            "truncated": map.truncated,
                        }),
                        error: None,
                    })
                }
                other => Ok(ToolResult {
                    ok: false,
                    content: serde_json::json!({}),
                    error: Some(format!("unknown action: {other}")),
                }),
            }
        })
        .await
        {
            Ok(res) => res,
            Err(join_err) => Err(anyhow::anyhow!("symbols tool worker panicked: {join_err}")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoMapFile {
    pub path: String,
    pub symbols: Vec<String>,
    /// Symbol count before the per-file cap, so the model can tell a large
    /// module from one with only a few declarations.
    pub total_symbols: usize,
    /// Why this file ranks where it does (source dir, symbol density, tests).
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoMap {
    pub files: Vec<RepoMapFile>,
    pub total_files: usize,
    pub truncated: bool,
}

/// A repo-wide outline ordered by likely relevance.
///
/// This is deliberately a structural map, not a semantic one: it gives the
/// model the shape of the workspace (`src/` before `tests/`, symbol-dense files
/// first) without needing an LSP or a tree-sitter grammar per language. The
/// per-file declarations come from the same heuristic as `outline`.
fn build_repo_map(root: &Path, max_symbols: usize) -> anyhow::Result<RepoMap> {
    const MAX_FILES_SCANNED: usize = 2_000;
    const MAX_SYMBOLS_PER_FILE: usize = 8;
    const CODE_EXTENSIONS: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "kts", "c", "cc",
        "cpp", "cxx", "h", "hpp", "cs", "rb", "php", "swift", "scala", "sh", "bash", "zsh",
    ];

    let mut builder = WalkBuilder::new(root);
    builder.standard_filters(true);
    builder.hidden(false);
    let walker = builder.build();

    let mut candidates: Vec<(String, Vec<OutlineItem>, String)> = Vec::new();
    let mut scanned = 0usize;
    for entry in walker {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !CODE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        if let Ok(meta) = path.metadata() {
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        // Vendored and generated trees dominate by volume and are never the
        // answer to "where is this behaviour implemented?".
        if rel.split('/').any(|part| {
            matches!(
                part,
                "node_modules" | "target" | "dist" | "build" | ".git" | "vendor" | "coverage"
            )
        }) {
            continue;
        }
        scanned += 1;
        if scanned > MAX_FILES_SCANNED {
            break;
        }
        let Ok(symbols) = extract_outline(path) else {
            continue;
        };
        if symbols.is_empty() {
            continue;
        }
        let reason = if rel.starts_with("src/") || rel.contains("/src/") {
            "source"
        } else if rel.starts_with("tests/")
            || rel.contains("/tests/")
            || rel.contains("test_")
            || rel.ends_with("_test.rs")
            || rel.ends_with(".test.ts")
            || rel.ends_with(".test.tsx")
        {
            "test"
        } else {
            "other"
        };
        candidates.push((rel, symbols, reason.to_string()));
    }

    // Source files first, then symbol density, then a stable path order so the
    // same repository always produces the same map.
    candidates.sort_by(|a, b| {
        let rank = |reason: &str| match reason {
            "source" => 0,
            "other" => 1,
            _ => 2,
        };
        rank(&a.2)
            .cmp(&rank(&b.2))
            .then_with(|| b.1.len().cmp(&a.1.len()))
            .then_with(|| a.0.cmp(&b.0))
    });

    let total_files = candidates.len();
    let mut files = Vec::new();
    let mut budget = max_symbols.max(1);
    for (path, symbols, reason) in candidates {
        if budget == 0 {
            break;
        }
        let total = symbols.len();
        let take = total.min(MAX_SYMBOLS_PER_FILE).min(budget);
        let names = symbols
            .into_iter()
            .take(take)
            .map(|item| item.name)
            .collect::<Vec<_>>();
        budget = budget.saturating_sub(names.len());
        files.push(RepoMapFile {
            path,
            symbols: names,
            total_symbols: total,
            reason,
        });
    }

    Ok(RepoMap {
        truncated: budget == 0 || files.len() < total_files,
        files,
        total_files,
    })
}

fn extract_outline(path: &Path) -> anyhow::Result<Vec<OutlineItem>> {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => return Err(anyhow::anyhow!("could not read file: {err}")),
    };

    let mut items = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
        {
            continue;
        }

        if let Some((kind, name)) = parse_declaration(trimmed) {
            items.push(OutlineItem {
                line: idx + 1,
                kind,
                name,
                signature: trimmed.chars().take(120).collect(),
            });
        }
    }

    Ok(items)
}

fn parse_declaration(line: &str) -> Option<(String, String)> {
    // Rust / Go / TypeScript / Python / Java common definitions
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    // Strip visibility modifiers: pub, export, async, static, default
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "pub" | "pub(crate)" | "pub(super)" | "export" | "default" | "async" | "static"
            | "const"
                if i + 1 < tokens.len() && tokens[i + 1] == "fn" =>
            {
                i += 1;
            }
            "pub" | "pub(crate)" | "pub(super)" | "export" | "default" | "async" | "static" => {
                i += 1;
            }
            _ => break,
        }
    }

    if i >= tokens.len() {
        return None;
    }

    let keyword = tokens[i];
    let next = tokens.get(i + 1).copied().unwrap_or("");

    match keyword {
        // Functions / Methods
        "fn" | "def" | "func" | "function" => {
            let name = next.split('(').next().unwrap_or(next);
            if !name.is_empty() {
                return Some(("function".to_string(), name.to_string()));
            }
        }
        // Structs / Classes
        "struct" | "class" => {
            let name = next.split('<').next().unwrap_or(next).trim_end_matches('{');
            if !name.is_empty() {
                return Some(("class/struct".to_string(), name.to_string()));
            }
        }
        // Interfaces / Traits
        "interface" | "trait" => {
            let name = next.split('<').next().unwrap_or(next).trim_end_matches('{');
            if !name.is_empty() {
                return Some(("interface/trait".to_string(), name.to_string()));
            }
        }
        // Enums
        "enum" => {
            let name = next.split('<').next().unwrap_or(next).trim_end_matches('{');
            if !name.is_empty() {
                return Some(("enum".to_string(), name.to_string()));
            }
        }
        // Types / Type aliases
        "type" => {
            let name = next
                .split('=')
                .next()
                .unwrap_or(next)
                .split('<')
                .next()
                .unwrap_or(next);
            if !name.is_empty() {
                return Some(("type".to_string(), name.to_string()));
            }
        }
        _ => {}
    }

    None
}

fn find_references(
    workspace_root: &Path,
    symbol: &str,
    max_results: usize,
) -> anyhow::Result<Vec<ReferenceItem>> {
    let pattern = format!(r"\b{}\b", regex::escape(symbol));
    let re = match regex::Regex::new(&pattern) {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };

    let mut builder = WalkBuilder::new(workspace_root);
    builder.standard_filters(true);
    builder.hidden(false);

    let walker = builder.build();
    let mut results = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Check size limit
        if let Ok(meta) = path.metadata() {
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue, // Ignore binary or unreadable
        };

        let rel_path = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                results.push(ReferenceItem {
                    path: rel_path.clone(),
                    line: idx + 1,
                    text: line.trim().chars().take(160).collect(),
                });
                if results.len() >= max_results {
                    return Ok(results);
                }
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mixd-symbols-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn repo_map_ranks_source_above_tests_and_skips_vendored_trees() {
        let root = temp_dir("map");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn alpha() {}\npub struct Beta;\nfn gamma() {}\n",
        )
        .unwrap();
        std::fs::write(root.join("tests/it.rs"), "fn only_test() {}\n").unwrap();
        std::fs::write(
            root.join("node_modules/pkg/index.js"),
            "function vendored() {}\n",
        )
        .unwrap();

        let map = build_repo_map(&root, 20).expect("map");
        let paths: Vec<&str> = map.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths[0], "src/lib.rs");
        assert!(
            !paths.iter().any(|p| p.contains("node_modules")),
            "vendored trees are not part of the map: {paths:?}"
        );
        assert!(paths.contains(&"tests/it.rs"));
        let lib = &map.files[0];
        assert_eq!(lib.reason, "source");
        assert!(lib.symbols.contains(&"alpha".to_string()));
        assert_eq!(lib.total_symbols, 3);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn repo_map_honours_the_symbol_budget() {
        let root = temp_dir("budget");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n",
        )
        .unwrap();
        let map = build_repo_map(&root, 2).expect("map");
        assert_eq!(map.files[0].symbols.len(), 2);
        assert!(map.truncated);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
