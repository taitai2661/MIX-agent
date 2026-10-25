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
         \"references\" with a symbol name to find everywhere it is used in the workspace."
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
                    "enum": ["outline", "references"],
                    "default": "outline",
                    "description": "outline: list symbol definitions in a file. references: search for symbol occurrences across workspace."
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
