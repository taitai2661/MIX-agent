//! Git-backed rollback coverage: the snapshot must restore tracked changes and
//! remove files the Run created, without touching untracked files that predated
//! it.

use std::path::Path;

use mix_agent_desktop::workspace_baseline;
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::Storage;

fn git(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-baseline-test-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

#[tokio::test]
async fn a_baseline_restores_tracked_and_removes_created_files() {
    let root = std::env::temp_dir().join(format!("mixd-baseline-repo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create repo");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(root.join("tracked.txt"), "one").expect("write");
    std::fs::write(root.join("pre-untracked.txt"), "keep me").expect("write");
    git(&root, &["add", "tracked.txt"]);
    git(&root, &["commit", "-qm", "initial"]);

    // An uncommitted parent change must be part of the baseline (`stash create`
    // is what makes the delegate/rollback see it).
    std::fs::write(root.join("tracked.txt"), "two").expect("write");

    let storage = temp_storage().await;
    let pool = storage.pool();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES ('conv-b', ?, '{}')")
        .bind(SINGLE_USER_ID)
        .execute(pool)
        .await
        .expect("insert conversation");
    let run_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, '{}', 'conv-b', 'completed', ?)",
    )
    .bind(run_id.to_string())
    .bind(SINGLE_USER_ID)
    .bind(run_id.to_string())
    .execute(pool)
    .await
    .expect("insert run");

    workspace_baseline::capture_and_store(pool, Some(&root.to_string_lossy()), run_id).await;

    // The Run changes the tracked file and creates a new one.
    std::fs::write(root.join("tracked.txt"), "three").expect("write");
    std::fs::write(root.join("created.txt"), "generated").expect("write");

    let changes = workspace_baseline::changes(pool, run_id)
        .await
        .expect("changes");
    assert!(changes.available);
    let paths: Vec<&str> = changes.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"tracked.txt"), "got {paths:?}");
    assert!(paths.contains(&"created.txt"), "got {paths:?}");

    let touched = workspace_baseline::revert(pool, run_id)
        .await
        .expect("revert");
    assert!(touched >= 2, "touched {touched}");
    assert_eq!(
        std::fs::read_to_string(root.join("tracked.txt")).expect("read"),
        "two"
    );
    assert!(!root.join("created.txt").exists(), "generated file removed");
    assert!(
        root.join("pre-untracked.txt").exists(),
        "pre-existing untracked file must survive"
    );

    let after = workspace_baseline::changes(pool, run_id)
        .await
        .expect("changes after revert");
    assert!(!after.available, "baseline is consumed by a revert");

    let _ = std::fs::remove_dir_all(&root);
}

/// `git diff --name-status -z` emits three fields for a rename. The rollback
/// must restore the old path and remove the new one; the old parser shifted
/// the list and left the renamed file behind.
#[tokio::test]
async fn a_rename_is_restored_to_the_old_path() {
    let root = std::env::temp_dir().join(format!("mixd-baseline-rename-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create repo");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(root.join("old.txt"), "content").expect("write");
    git(&root, &["add", "old.txt"]);
    git(&root, &["commit", "-qm", "initial"]);

    let storage = temp_storage().await;
    let pool = storage.pool();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES ('conv-r', ?, '{}')")
        .bind(SINGLE_USER_ID)
        .execute(pool)
        .await
        .expect("insert conversation");
    let run_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, '{}', 'conv-r', 'completed', ?)",
    )
    .bind(run_id.to_string())
    .bind(SINGLE_USER_ID)
    .bind(run_id.to_string())
    .execute(pool)
    .await
    .expect("insert run");

    workspace_baseline::capture_and_store(pool, Some(&root.to_string_lossy()), run_id).await;
    git(&root, &["mv", "old.txt", "new.txt"]);

    let changes = workspace_baseline::changes(pool, run_id)
        .await
        .expect("changes");
    let paths: Vec<&str> = changes.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"old.txt"), "got {paths:?}");
    assert!(paths.contains(&"new.txt"), "got {paths:?}");

    workspace_baseline::revert(pool, run_id)
        .await
        .expect("revert");
    assert_eq!(
        std::fs::read_to_string(root.join("old.txt")).expect("old path restored"),
        "content"
    );
    assert!(!root.join("new.txt").exists(), "new path removed");

    let _ = std::fs::remove_dir_all(&root);
}
