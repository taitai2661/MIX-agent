//! Git repository inspection tool (read-only).
//!
//! Provides first-class git discovery for the agent:
//!  - `status`: Lists branch and working-tree changes (modified, untracked, deleted).
//!  - `diff`: Retrieves unified diff of unstaged, staged, or single-file changes.

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
        "Inspect Git repository state. Use action: \"status\" to see branch name and modified/untracked files. \
         Use action: \"diff\" to inspect unified diffs of unstaged or staged changes."
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
                    "enum": ["status", "diff"],
                    "default": "status",
                    "description": "status: check repo changes. diff: inspect unified diff."
                },
                "scope": {
                    "type": "string",
                    "enum": ["unstaged", "staged", "file"],
                    "default": "unstaged",
                    "description": "diff scope: unstaged (working directory), staged (index), or file."
                },
                "path": {
                    "type": "string",
                    "description": "Optional file path to limit the diff to."
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
