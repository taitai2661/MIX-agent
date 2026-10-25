//! Read-only workspace checks without a shell.
//!
//! Mirrors Docker's `workspace_check` (`runners/execution/main.py`): fixed
//! commands only, never model-supplied text as a command. Kinds:
//! `git_status`, `git_diff_check`, `python_syntax` (requires a `.py` path).

use async_trait::async_trait;
use tokio::process::Command as TokioCommand;

use super::files::FilesTool;
use super::{Tool, ToolContext, ToolResult};

const CHECK_TIMEOUT_SECS: u64 = 20;
const MAX_OUTPUT_BYTES: usize = 64_000;

#[derive(Default)]
pub struct WorkspaceCheckTool;

fn truncate_output(text: &str) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text.to_string();
    }
    let mut cut = MAX_OUTPUT_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}… [truncated]", &text[..cut])
}

async fn run_git(root: &std::path::Path, kind: &str) -> anyhow::Result<ToolResult> {
    let mut args: Vec<&str> = vec![
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
    if kind == "git_status" {
        args.extend([
            "status",
            "--short",
            "--untracked-files=no",
            "--ignore-submodules=all",
        ]);
    } else {
        args.extend([
            "diff",
            "--check",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=all",
        ]);
    }
    let mut cmd = TokioCommand::new("git");
    cmd.args(&args).current_dir(root).kill_on_drop(true);
    // Restricted env, mirroring the Docker runner: no system/global git config,
    // no optional locks, minimal PATH/HOME.
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    cmd.env("HOME", "/tmp");
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(CHECK_TIMEOUT_SECS),
        cmd.output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("workspace check timed out"))?
    .map_err(|e| anyhow::anyhow!("failed to run git: {e}"))?;

    let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let code = output.status.code().unwrap_or(-1);
    Ok(ToolResult {
        ok: output.status.success(),
        content: serde_json::json!({
            "kind": kind,
            "ok": output.status.success(),
            "exit_code": code,
            "output": truncate_output(&combined),
        }),
        error: if output.status.success() {
            None
        } else {
            Some(format!("git exited with {code}"))
        },
    })
}

async fn check_python_syntax(
    root_path: Option<String>,
    additional: Vec<String>,
    path: &str,
) -> anyhow::Result<ToolResult> {
    if !path.ends_with(".py") {
        anyhow::bail!("A Python file path is required");
    }
    let workspace_root = FilesTool::resolve_context_root(root_path.as_deref())?;
    let (resolved, _) = FilesTool::resolve_target_in(&workspace_root, &additional, path)?;
    if !resolved.is_file() {
        anyhow::bail!("file not found: {path}");
    }
    // Validate via the system interpreter when present (`ast.parse`
    // equivalent). Without python3 there is nothing trustworthy to check with,
    // so fail loudly rather than claim success.
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(CHECK_TIMEOUT_SECS),
        TokioCommand::new("python3")
            .args(["-m", "py_compile"])
            .arg(&resolved)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("workspace check timed out"))?
    .map_err(|e| anyhow::anyhow!("python3 is required for python_syntax: {e}"))?;
    let ok = out.status.success();
    let detail = if ok {
        String::new()
    } else {
        let mut s = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if !stderr.is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&stderr);
        }
        truncate_output(&s)
    };
    Ok(ToolResult {
        ok,
        content: serde_json::json!({ "kind": "python_syntax", "path": path, "ok": ok, "output": detail }),
        error: if ok {
            None
        } else {
            Some("python syntax error".to_string())
        },
    })
}

#[async_trait]
impl Tool for WorkspaceCheckTool {
    fn id(&self) -> &str {
        "workspace_check"
    }
    fn name(&self) -> &str {
        "Workspace check"
    }
    fn description(&self) -> &str {
        "Run a fixed read-only workspace check without a shell. Kinds: git_status, git_diff_check, python_syntax. Python syntax requires a path."
    }
    /// Read-only: safe to allow without approval, like `glob`/`grep`.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("kind")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["git_status", "git_diff_check", "python_syntax"] },
                "path": { "type": "string" }
            },
            "required": ["kind"],
            "additionalProperties": false
        })
    }
    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let kind = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("kind is required"))?;
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        match kind {
            "python_syntax" => {
                if path.is_empty() {
                    anyhow::bail!("A Python file path is required");
                }
                check_python_syntax(ctx.root_path, ctx.additional_paths, path).await
            }
            "git_status" | "git_diff_check" => {
                if !path.is_empty() {
                    anyhow::bail!("Unknown check or unexpected path");
                }
                let root = FilesTool::resolve_context_root(ctx.root_path.as_deref())?;
                run_git(&root, kind).await
            }
            _ => anyhow::bail!("Unknown check or unexpected path"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_in(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: Some(dir.to_string_lossy().to_string()),
            additional_paths: Vec::new(),
            approval_token: None,
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mixd-wscheck-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn rejects_unknown_kind() {
        let tool = WorkspaceCheckTool;
        let dir = temp_dir("unknown");
        let err = tool
            .invoke(
                ctx_in(&dir),
                serde_json::json!({"kind": "git_status; touch bad"}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Unknown check"));
    }

    #[tokio::test]
    async fn git_kinds_reject_a_path() {
        let tool = WorkspaceCheckTool;
        let dir = temp_dir("gitpath");
        let err = tool
            .invoke(
                ctx_in(&dir),
                serde_json::json!({"kind": "git_status", "path": "a.py"}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unexpected path"));
    }

    #[tokio::test]
    async fn python_syntax_requires_a_py_path() {
        let tool = WorkspaceCheckTool;
        let dir = temp_dir("pypath");
        let err = tool
            .invoke(ctx_in(&dir), serde_json::json!({"kind": "python_syntax"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Python file path"));
        let err = tool
            .invoke(
                ctx_in(&dir),
                serde_json::json!({"kind": "python_syntax", "path": "notes.txt"}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Python file path"));
    }

    #[tokio::test]
    async fn python_syntax_blocks_traversal() {
        let tool = WorkspaceCheckTool;
        let dir = temp_dir("traversal");
        let err = tool
            .invoke(
                ctx_in(&dir),
                serde_json::json!({"kind": "python_syntax", "path": "../outside.py"}),
            )
            .await
            .unwrap_err();
        let message = err.to_string();
        let message_lower = message.to_ascii_lowercase();
        assert!(
            message_lower.contains("outside the workspace") || message_lower.contains("escapes"),
            "unexpected error for traversal rejection: {message}"
        );
    }

    #[tokio::test]
    async fn python_syntax_accepts_valid_file() {
        let tool = WorkspaceCheckTool;
        let dir = temp_dir("pyok");
        std::fs::write(dir.join("ok.py"), "x = 1\n").unwrap();
        let result = tool
            .invoke(
                ctx_in(&dir),
                serde_json::json!({"kind": "python_syntax", "path": "ok.py"}),
            )
            .await
            .expect("invoke");
        assert!(result.ok, "valid python must pass: {:?}", result.error);
        assert_eq!(result.content["kind"], "python_syntax");
    }

    #[tokio::test]
    async fn python_syntax_rejects_broken_file() {
        let tool = WorkspaceCheckTool;
        let dir = temp_dir("pybad");
        std::fs::write(dir.join("bad.py"), "def broken(:\n").unwrap();
        let result = tool
            .invoke(
                ctx_in(&dir),
                serde_json::json!({"kind": "python_syntax", "path": "bad.py"}),
            )
            .await
            .expect("invoke");
        assert!(!result.ok, "broken python must fail");
    }

    #[tokio::test]
    async fn git_status_runs_in_a_repo() {
        let tool = WorkspaceCheckTool;
        let dir = temp_dir("gitrepo");
        // Init a repo; skip when git is unavailable.
        if tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .is_err()
        {
            return;
        }
        let result = tool
            .invoke(ctx_in(&dir), serde_json::json!({"kind": "git_status"}))
            .await
            .expect("invoke");
        assert!(result.ok, "git status must succeed: {:?}", result.error);
        assert_eq!(result.content["kind"], "git_status");
    }

    #[test]
    fn reads_are_free() {
        let tool = WorkspaceCheckTool;
        assert!(!tool.is_sensitive(&serde_json::json!({"kind": "git_status"})));
    }
}
