//! Isolated git worktrees for agent delegation.
//!
//! Extracted from `lib.rs` to keep the Tauri wiring readable: stale-worktree
//! pruning, private-index base commits, and worktree create/diff/cleanup.
//! All git I/O is timeout-bounded and never buffers unbounded patches.

/// The single base directory for every delegate checkout.
///
/// Both the startup pruner (`cleanup_stale_worktrees`) and the creator
/// (`prepare_delegate_worktree`) must agree on this path. They used to disagree
/// (`data_dir/worktrees` vs `dirs::data_dir()/APP_IDENTIFIER/worktrees`), so with
/// a `MIX_DATA_DIR` override the pruner watched an empty directory while the
/// creator leaked a checkout per fan-out. Both now derive from the same app-data
/// directory (`crate::cli::data_dir()` honours `MIX_DATA_DIR`). The GUI also
/// refuses to start via `cli::verify_data_dir` when its own resolver disagrees
/// with that canonical path, so the pruner and the creator cannot diverge.
fn worktrees_dir(app_data: &std::path::Path) -> std::path::PathBuf {
    app_data.join("worktrees")
}

/// Remove delegate worktrees left behind by a crash or a killed process.
///
/// The git metadata is pruned by `prepare_delegate_worktree` in the repository
/// that owns it; this removes the checkout directories themselves. Only stale
/// entries are touched so a delegate running in another window is safe.
pub async fn cleanup_stale_worktrees(data_dir: &std::path::Path) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    let base = worktrees_dir(data_dir);
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

/// Pathspecs excluded from the delegate's `git add -A`.
///
/// A delegate base commit stages the *untracked* workspace so the child can see
/// newly created files. `git add -A` also picked up `.env`, private keys and
/// credential files, which then lived in the private git object store (and in
/// the diff a parent is asked to merge). These exclusions shrink that blast
/// radius. They are a best-effort deny-list, not a secret scanner: secrets
/// already committed to the repository, or stored under an unusual name, are
/// still reachable by the delegate.
const SENSITIVE_PATHSPECS: &[&str] = &[
    ":(exclude).env",
    ":(exclude)*.env",
    ":(exclude).env.*",
    ":(exclude)*/.env",
    ":(exclude)*/.env.*",
    ":(exclude)**/.env",
    ":(exclude)**/.env.*",
    // A bare `*.pem` only matches the repository root: git pathspecs do not
    // cross `/` without `**`. Without these, `config/tls.pem` and
    // `keys/id_rsa` were staged into the delegate's private object store.
    ":(exclude)*.pem",
    ":(exclude)**/*.pem",
    ":(exclude)*.key",
    ":(exclude)**/*.key",
    ":(exclude)*.p12",
    ":(exclude)**/*.p12",
    ":(exclude)*.pfx",
    ":(exclude)**/*.pfx",
    ":(exclude)*.keystore",
    ":(exclude)**/*.keystore",
    ":(exclude)*.jks",
    ":(exclude)**/*.jks",
    ":(exclude)*.ppk",
    ":(exclude)**/*.ppk",
    ":(exclude)id_rsa",
    ":(exclude)**/id_rsa",
    ":(exclude)id_ed25519",
    ":(exclude)**/id_ed25519",
    ":(exclude).npmrc",
    ":(exclude)**/.npmrc",
    ":(exclude).yarnrc",
    ":(exclude)**/.yarnrc",
    ":(exclude).pypirc",
    ":(exclude)**/.pypirc",
    ":(exclude).netrc",
    ":(exclude)**/.netrc",
    ":(exclude)credentials",
    ":(exclude)credentials.json",
    ":(exclude)credentials.*",
    ":(exclude)**/credentials",
    ":(exclude)**/credentials.json",
    ":(exclude)**/credentials.*",
    ":(exclude)token.json",
    ":(exclude)**/token.json",
    ":(exclude)tokens.json",
    ":(exclude)**/tokens.json",
];

/// `git add -A` with the sensitive-file exclusions applied. Both the private
/// index (base commit) and the worktree (collected diff) use this so the two
/// trees never disagree about what was staged.
fn add_all_args() -> Vec<&'static str> {
    let mut args = vec!["add", "-A", "--", "."];
    args.extend_from_slice(SENSITIVE_PATHSPECS);
    args
}

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
        let add_args = add_all_args();
        let added = run_git_env(root, &add_args, &index_env).await.ok()?;
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

    // Shared with the startup pruner: a `MIX_DATA_DIR` override used to leave
    // these checkouts invisible to cleanup.
    let base_dir = worktrees_dir(&crate::cli::data_dir());
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
    // Stage everything first so newly created files appear in the patch, but
    // leave .env / key material out of the object store and the returned diff.
    let add_args = add_all_args();
    let _ = run_git(worktree, &add_args).await;
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

/// Confirmation input for the parent's merge decision: does `diff` still
/// apply cleanly on `root`?
///
/// Read-only (`--check`): nothing is written either way. The caller keeps the
/// temporary worktree's diff and asks the parent to apply it deliberately —
/// this only reports whether that application would succeed at this instant.
pub(crate) async fn check_patch_applies(root: &str, diff: &str) -> bool {
    use tokio::io::AsyncWriteExt as _;

    if diff.trim().is_empty() {
        return true;
    }
    let mut command = git_command(std::path::Path::new(root), &["apply", "--check", "-"]);
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    // Take the pipe out first so the write and the wait below do not borrow
    // `child` at the same time.
    let stdin = child.stdin.take();
    let status = match tokio::time::timeout(GIT_TIMEOUT, async move {
        if let Some(mut stdin) = stdin {
            // A bounded write: the diff itself was already capped at
            // `DELEGATE_DIFF_LIMIT` when it was collected.
            stdin.write_all(diff.as_bytes()).await.ok();
        }
        child.wait().await
    })
    .await
    {
        Ok(Ok(status)) => status,
        _ => return false,
    };
    status.success()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mix-worktree-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(dir.join("src")).expect("create temp repo");
        std::fs::create_dir_all(dir.join("config")).expect("create nested dir");
        std::fs::create_dir_all(dir.join("keys")).expect("create nested keys dir");
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").expect("write source");
        std::fs::write(dir.join(".env"), "API_TOKEN=secret\n").expect("write .env");
        std::fs::write(dir.join("deploy.key"), "private key\n").expect("write key");
        // Nested secret files must be excluded too: a bare `*.key` pathspec
        // never crosses a `/`, so these used to land in the delegate index.
        std::fs::write(dir.join("config").join("tls.pem"), "-----BEGIN KEY-----\n")
            .expect("write nested pem");
        std::fs::write(dir.join("keys").join("id_rsa"), "private key\n").expect("write nested key");
        std::fs::write(dir.join("config").join(".env"), "NESTED=secret\n")
            .expect("write nested .env");
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .expect("run git init");
        assert!(init.success());
        dir
    }

    #[test]
    fn add_all_args_carry_the_sensitive_exclusions() {
        let args = add_all_args();
        assert_eq!(&args[..4], &["add", "-A", "--", "."]);
        for needle in [".env", ".pem", ".key", ".npmrc", "credentials"] {
            assert!(
                args.iter().any(|arg| arg.contains(needle)),
                "no exclusion for {needle}"
            );
        }
    }

    #[tokio::test]
    async fn staging_a_delegate_base_skips_dotenv_and_private_keys() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            // git is a runtime dependency of delegation, not of the test host.
            return;
        }
        let dir = temp_repo();
        let staged = run_git(&dir, &add_all_args()).await.expect("git add");
        assert!(staged.status.success());
        let names = run_git(&dir, &["diff", "--cached", "--name-only"])
            .await
            .expect("git diff");
        let names = String::from_utf8_lossy(&names.stdout);
        assert!(
            names.contains("src/main.rs"),
            "normal files are staged: {names}"
        );
        assert!(
            !names.contains(".env"),
            ".env leaked into the index: {names}"
        );
        assert!(
            !names.contains("deploy.key"),
            "private key leaked into the index: {names}"
        );
        assert!(
            !names.contains("config/tls.pem"),
            "nested PEM leaked into the index: {names}"
        );
        assert!(
            !names.contains("keys/id_rsa"),
            "nested private key leaked into the index: {names}"
        );
        assert!(
            !names.contains("config/.env"),
            "nested .env leaked into the index: {names}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
