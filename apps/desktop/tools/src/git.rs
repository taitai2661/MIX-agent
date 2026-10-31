//! Git repository inspection tool (read-only).
//!
//! Provides first-class git discovery for the agent:
//!  - `status`: Lists branch and working-tree changes (modified, untracked, deleted).
//!  - `diff`: Retrieves unified diff of unstaged, staged, or single-file changes.
//!  - `log` / `show` / `blame`: history, one commit, and line provenance.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

use super::files::FilesTool;
use super::{Tool, ToolContext, ToolResult};

const GIT_TIMEOUT_SECS: u64 = 15;
const MAX_DIFF_BYTES: usize = 128_000;

#[derive(Default)]
pub struct GitTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusEntry {
    pub path: String,
    pub status: String,
}

#[async_trait]
impl Tool for GitTool {
    fn id(&self) -> &str {
        "git"
    }

    fn name(&self) -> &str {
        "Git"
    }

    fn description(&self) -> &str {
        "Inspect Git repository state. \"status\" lists branch and working-tree changes; \
         \"diff\" shows a unified diff; \"log\" lists recent commits; \"show\" displays one \
         commit (default HEAD); \"blame\" shows line provenance for a file."
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
                    "enum": ["status", "diff", "log", "show", "blame"],
                    "default": "status",
                    "description": "status: repo changes. diff: unified diff. log: recent commits. show: one commit. blame: line provenance."
                },
                "scope": {
                    "type": "string",
                    "enum": ["unstaged", "staged", "file"],
                    "default": "unstaged",
                    "description": "diff scope: unstaged (working directory), staged (index), or file."
                },
                "path": {
                    "type": "string",
                    "description": "Optional file path to limit the output to (diff, log, blame)."
                },
                "rev": {
                    "type": "string",
                    "description": "Revision for action \"show\" (default HEAD), e.g. HEAD~2 or a sha."
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "description": "How many commits \"log\" returns (1-50)."
                }
            }
        })
    }

    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let workspace_root = FilesTool::resolve_context_root(ctx.root_path.as_deref())?;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status");

        match action {
            "status" => run_git_status(&workspace_root).await,
            "diff" => {
                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unstaged");
                let path = args.get("path").and_then(|v| v.as_str());
                run_git_diff(&workspace_root, scope, path).await
            }
            "log" => {
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    .clamp(1, 50);
                let path = args.get("path").and_then(|v| v.as_str());
                run_git_log(&workspace_root, limit, path).await
            }
            "show" => {
                let rev = args.get("rev").and_then(|v| v.as_str()).unwrap_or("HEAD");
                run_git_show(&workspace_root, rev).await
            }
            "blame" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .filter(|p| !p.trim().is_empty())
                    .ok_or_else(|| anyhow::anyhow!("path is required for action: blame"))?;
                run_git_blame(&workspace_root, path).await
            }
            other => Ok(ToolResult {
                ok: false,
                content: serde_json::json!({}),
                error: Some(format!("unknown git action: {other}")),
            }),
        }
    }
}

async fn run_git_status(root: &Path) -> anyhow::Result<ToolResult> {
    let output = match exec_git(root, &["status", "--porcelain=v1", "-uall"]).await {
        Ok(out) => out,
        Err(err) => {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "available": false }),
                error: Some(format!("git status failed: {err}")),
            });
        }
    };

    if !output.status.success() {
        return Ok(ToolResult {
            ok: false,
            content: serde_json::json!({ "available": false }),
            error: Some("not a git repository or git error".to_string()),
        });
    }

    let branch = exec_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
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
        let (xy, rest) = line.split_at(3);
        let path = rest.split(" -> ").last().unwrap_or(rest).trim();
        entries.push(GitStatusEntry {
            path: path.to_string(),
            status: xy.trim().to_string(),
        });
    }

    Ok(ToolResult {
        ok: true,
        content: serde_json::json!({
            "available": true,
            "branch": branch,
            "clean": entries.is_empty(),
            "changes": entries,
        }),
        error: None,
    })
}

async fn run_git_diff(
    root: &Path,
    scope: &str,
    path_filter: Option<&str>,
) -> anyhow::Result<ToolResult> {
    let mut args: Vec<&str> = vec!["diff", "--no-ext-diff", "--no-color"];
    match scope {
        "staged" => args.push("--cached"),
        "file" => {}
        _ => {}
    }

    if let Some(p) = path_filter {
        if !p.trim().is_empty() {
            // Verify path does not escape workspace
            FilesTool::safe_join(root, p)?;
            args.push("--");
            args.push(p);
        }
    }

    let output = match exec_git(root, &args).await {
        Ok(out) => out,
        Err(err) => {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "available": false }),
                error: Some(format!("git diff failed: {err}")),
            });
        }
    };

    let diff_text = String::from_utf8_lossy(&output.stdout);
    let (unified_diff, truncated) = if diff_text.len() > MAX_DIFF_BYTES {
        (
            format!("{}… [truncated]", &diff_text[..MAX_DIFF_BYTES]),
            true,
        )
    } else {
        (diff_text.to_string(), false)
    };

    Ok(ToolResult {
        ok: true,
        content: serde_json::json!({
            "available": true,
            "scope": scope,
            "path": path_filter,
            "diff": unified_diff,
            "truncated": truncated,
        }),
        error: None,
    })
}

/// Reject a revision that would be parsed as an option (`--help`, `-c ...`).
fn safe_rev(rev: &str) -> anyhow::Result<&str> {
    let rev = rev.trim();
    if rev.is_empty() || rev.starts_with('-') {
        anyhow::bail!("invalid revision: {rev:?}");
    }
    if !rev
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '~' | '^' | '-' | '_'))
    {
        anyhow::bail!("invalid revision: {rev:?}");
    }
    Ok(rev)
}

async fn run_git_log(
    root: &Path,
    limit: u64,
    path_filter: Option<&str>,
) -> anyhow::Result<ToolResult> {
    let count = format!("-n{limit}");
    let mut args: Vec<&str> = vec!["log", "--oneline", "--decorate", "--no-color", &count];
    if let Some(path) = path_filter.filter(|p| !p.trim().is_empty()) {
        FilesTool::safe_join(root, path)?;
        args.push("--");
        args.push(path);
    }
    let output = match exec_git(root, &args).await {
        Ok(out) => out,
        Err(err) => {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "available": false }),
                error: Some(format!("git log failed: {err}")),
            });
        }
    };
    if !output.status.success() {
        return Ok(ToolResult {
            ok: false,
            content: serde_json::json!({ "available": false }),
            error: Some("not a git repository or git error".to_string()),
        });
    }
    Ok(ToolResult {
        ok: true,
        content: serde_json::json!({
            "available": true,
            "log": clip(String::from_utf8_lossy(&output.stdout).to_string()),
        }),
        error: None,
    })
}

async fn run_git_show(root: &Path, rev: &str) -> anyhow::Result<ToolResult> {
    let rev = safe_rev(rev)?;
    let output = match exec_git(root, &["show", "--no-color", "--stat", rev]).await {
        Ok(out) => out,
        Err(err) => {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "available": false }),
                error: Some(format!("git show failed: {err}")),
            });
        }
    };
    if !output.status.success() {
        return Ok(ToolResult {
            ok: false,
            content: serde_json::json!({ "available": true, "rev": rev }),
            error: Some(String::from_utf8_lossy(&output.stderr).to_string()),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(ToolResult {
        ok: true,
        content: serde_json::json!({
            "available": true,
            "rev": rev,
            "commit": clip(text),
        }),
        error: None,
    })
}

async fn run_git_blame(root: &Path, path: &str) -> anyhow::Result<ToolResult> {
    FilesTool::safe_join(root, path)?;
    let output = match exec_git(root, &["blame", "--no-color", "--date=short", path]).await {
        Ok(out) => out,
        Err(err) => {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "available": false }),
                error: Some(format!("git blame failed: {err}")),
            });
        }
    };
    if !output.status.success() {
        return Ok(ToolResult {
            ok: false,
            content: serde_json::json!({ "available": true, "path": path }),
            error: Some(String::from_utf8_lossy(&output.stderr).to_string()),
        });
    }
    Ok(ToolResult {
        ok: true,
        content: serde_json::json!({
            "available": true,
            "path": path,
            "blame": clip(String::from_utf8_lossy(&output.stdout).to_string()),
        }),
        error: None,
    })
}

/// Keep history output inside the same budget as a diff.
fn clip(text: String) -> String {
    if text.len() <= MAX_DIFF_BYTES {
        return text;
    }
    let cut = text
        .char_indices()
        .take_while(|(index, _)| *index < MAX_DIFF_BYTES)
        .last()
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    format!("{}… [truncated]", &text[..cut])
}

async fn exec_git(root: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let mut base_args = vec![
        "--no-optional-locks",
        "-c",
        "core.pager=cat",
        "-c",
        "diff.external=",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
    ];
    base_args.extend_from_slice(args);

    let mut cmd = TokioCommand::new("git");
    cmd.args(&base_args).current_dir(root).kill_on_drop(true);

    let child = cmd.output();
    tokio::time::timeout(Duration::from_secs(GIT_TIMEOUT_SECS), child)
        .await
        .map_err(|_| anyhow::anyhow!("git operation timed out"))?
        .map_err(|e| anyhow::anyhow!("git failed: {e}"))
}
