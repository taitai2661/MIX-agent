//! Isolated git worktrees for agent delegation.
//!
//! Extracted from `lib.rs` to keep the Tauri wiring readable: stale-worktree
//! pruning, private-index base commits, and worktree create/diff/cleanup.
//! All git I/O is timeout-bounded and never buffers unbounded patches.

//!
/// Remove delegate worktrees left behind by a crash or a killed process.
///
/// The git metadata is pruned by `prepare_delegate_worktree` in the repository
/// that owns it; this removes the checkout directories themselves. Only stale
/// entries are touched so a delegate running in another window is safe.
pub async fn cleanup_stale_worktrees(data_dir: &std::path::Path) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    let base = data_dir.join("worktrees");
    let Ok(mut entries) = tokio::fs::read_dir(&base).await else {
        return;
    };
    let now = std::time::SystemTime::now();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let stale = entry
            .metadata()
            .await
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > MAX_AGE);
        if stale {
            let _ = tokio::fs::remove_dir_all(&path).await;
        }
    }
}

/// Git helpers for isolated delegation. Kept outside the trait impl so they can
/// be tested without an engine.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Same cap the transcript uses for a delegated patch.
const DELEGATE_DIFF_LIMIT: usize = 128 * 1024;

fn git_command(cwd: &std::path::Path, args: &[&str]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(cwd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    command
}

async fn run_git(cwd: &std::path::Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let mut command = git_command(cwd, args);
    command.stdout(std::process::Stdio::piped());
    tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| anyhow::anyhow!("git {} timed out", args.join(" ")))?
        .map_err(Into::into)
}

/// Run git with extra environment variables (used for a private index file).
async fn run_git_env(
    cwd: &std::path::Path,
    args: &[&str],
    envs: &[(&str, &std::ffi::OsStr)],
) -> anyhow::Result<std::process::Output> {
    let mut command = git_command(cwd, args);
    for (key, value) in envs {
        command.env(*key, *value);
    }
    command.stdout(std::process::Stdio::piped());
    tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| anyhow::anyhow!("git {} timed out", args.join(" ")))?
        .map_err(Into::into)
}

/// Read at most `limit` bytes of git output, killing the child instead of
/// buffering a multi-megabyte binary patch in memory.
async fn run_git_limited(
    cwd: &std::path::Path,
    args: &[&str],
    limit: usize,
) -> anyhow::Result<Option<String>> {
    use tokio::io::AsyncReadExt as _;

    let mut command = git_command(cwd, args);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("git produced no stdout"))?;
    let mut data = Vec::new();
    let read = async {
        let mut limited = (&mut stdout).take((limit as u64).saturating_add(1));
        limited.read_to_end(&mut data).await?;
        Ok::<bool, std::io::Error>(data.len() > limit)
    };
    let truncated = match tokio::time::timeout(GIT_TIMEOUT, read).await {
        Ok(result) => result?,
        Err(_) => {
            let _ = child.kill().await;
            anyhow::bail!("git {} timed out", args.join(" "));
        }
    };
    if truncated {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    if data.is_empty() {
        return Ok(None);
    }
    if data.len() > limit {
        let mut cut = limit;
        while cut > 0 && std::str::from_utf8(&data[..cut]).is_err() {
            cut -= 1;
        }
        data.truncate(cut);
    }
    let mut text = String::from_utf8_lossy(&data).into_owned();
    if truncated {
        text.push_str("\n… (diff truncated)");
    }
    Ok(Some(text))
}

/// Build a commit that contains the parent worktree *including* untracked
/// files, without touching the parent's real index.
///
/// `git stash create` only records tracked paths, so a delegate used to open a
/// checkout where a new (untracked) file simply did not exist. A private
/// `GIT_INDEX_FILE` lets `git add -A` capture those paths and `write-tree`
/// turn the result into a commit the temporary worktree can check out.
pub(crate) async fn delegate_base_commit(root: &std::path::Path, head: &str) -> Option<String> {
    let index = std::env::temp_dir().join(format!(
        "mix-agent-delegate-index-{}.idx",
        uuid::Uuid::new_v4().simple()
    ));
    let index_env = [("GIT_INDEX_FILE", index.as_os_str())];
    let result = async {
        let read = run_git_env(root, &["read-tree", head], &index_env)
            .await
            .ok()?;
        if !read.status.success() {
            return None;
        }
        let added = run_git_env(root, &["add", "-A"], &index_env).await.ok()?;
        if !added.status.success() {
            return None;
        }
        let tree = run_git_env(root, &["write-tree"], &index_env).await.ok()?;
        if !tree.status.success() {
            return None;
        }
        let tree_oid = String::from_utf8_lossy(&tree.stdout).trim().to_string();
        if tree_oid.is_empty() {
            return None;
        }
        let head_tree = run_git(root, &["rev-parse", &format!("{head}^{{tree}}")])
            .await
            .ok()?;
        if head_tree.status.success()
            && String::from_utf8_lossy(&head_tree.stdout).trim() == tree_oid
        {
            return Some(head.to_string());
        }
        let identity = [
            ("GIT_AUTHOR_NAME", "MIX-agent"),
            ("GIT_AUTHOR_EMAIL", "mix@localhost"),
            ("GIT_COMMITTER_NAME", "MIX-agent"),
            ("GIT_COMMITTER_EMAIL", "mix@localhost"),
        ];
        let mut args = vec!["commit-tree", tree_oid.as_str(), "-p", head, "-m"];
        args.push("MIX-agent delegate base");
        let commit = run_git(root, &args).await.ok()?;
        // Re-run with the identity only when the host has none configured.
        let commit = if commit.status.success() {
            commit
        } else {
            let envs: Vec<(&str, &std::ffi::OsStr)> = identity
                .iter()
                .map(|(key, value)| (*key, std::ffi::OsStr::new(*value)))
                .collect();
            run_git_env(root, &args, &envs).await.ok()?
        };
        if !commit.status.success() {
            return None;
        }
        let oid = String::from_utf8_lossy(&commit.stdout).trim().to_string();
        (!oid.is_empty()).then_some(oid)
    }
    .await;
    let _ = tokio::fs::remove_file(&index).await;
    result
}

pub(crate) async fn prepare_delegate_worktree(
    root: &str,
    run_id: uuid::Uuid,
) -> Option<(std::path::PathBuf, String)> {
    let root_path = std::path::PathBuf::from(root);
    if !root_path.is_dir() {
        return None;
    }
    let inside = run_git(&root_path, &["rev-parse", "--is-inside-work-tree"])
        .await
        .ok()?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return None;
    }
    // Remove metadata for worktrees whose directory was deleted by a crash.
    let _ = run_git(&root_path, &["worktree", "prune"]).await;
    let head = run_git(&root_path, &["rev-parse", "HEAD"]).await.ok()?;
    if !head.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if head.is_empty() {
        return None;
    }
    let base = delegate_base_commit(&root_path, &head)
        .await
        .unwrap_or(head);

    let base_dir = dirs::data_dir()?
        .join(crate::APP_IDENTIFIER)
        .join("worktrees");
    if let Err(err) = tokio::fs::create_dir_all(&base_dir).await {
        tracing::warn!(?err, "could not create the delegate worktree directory");
        return None;
    }
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let worktree = base_dir.join(format!("{run_id}-{}", &suffix[..8]));
    let added = run_git(
        &root_path,
        &[
            "worktree",
            "add",
            "--detach",
            &worktree.to_string_lossy(),
            &base,
        ],
    )
    .await
    .ok()?;
    if !added.status.success() {
        tracing::warn!(
            stderr = %String::from_utf8_lossy(&added.stderr),
            "git worktree add failed; delegate will run in the parent workspace"
        );
        return None;
    }
    Some((worktree, base))
}

pub(crate) async fn collect_worktree_diff(
    worktree: &std::path::Path,
    base: &str,
) -> (Option<String>, Vec<String>) {
    // Stage everything first so newly created files appear in the patch.
    let _ = run_git(worktree, &["add", "-A"]).await;
    let changed = run_git(worktree, &["diff", "--cached", "--name-only", base])
        .await
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_string)
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let diff = run_git_limited(
        worktree,
        &["diff", "--cached", "--binary", base],
        DELEGATE_DIFF_LIMIT,
    )
    .await
    .ok()
    .flatten()
    .filter(|value| !value.trim().is_empty());
    (diff, changed)
}

pub(crate) async fn cleanup_delegate_worktree(root: &str, worktree: &std::path::Path) {
    let parent = std::path::PathBuf::from(root);
    let removed = run_git(
        &parent,
        &["worktree", "remove", "--force", &worktree.to_string_lossy()],
    )
    .await;
    if !removed
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        let _ = tokio::fs::remove_dir_all(worktree).await;
    }
}
