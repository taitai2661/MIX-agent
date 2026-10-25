//! Integration coverage for the instruction-file fallback chain.
//!
//! A run should pick up whatever instructions a repository actually keeps:
//! `AGENTS.md` first (the cross-tool standard), `CLAUDE.md` when that is what
//! exists, and the user's global file even for a conversation with no project
//! attached. Since the folder-first refactor every project is rooted at a
//! real directory, so the chain always has a root to start from.

use mix_agent_desktop::commands::projects::{load_project_runtime_with, InstructionConfig};
use mixd_storage::records::ProjectData;
use mixd_storage::Storage;
use uuid::Uuid;

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-project-runtime-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mixd-project-root-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn seeded_project_data(root: &std::path::Path) -> ProjectData {
    ProjectData {
        name: "Demo".into(),
        instructions: String::new(),
        root_path: root.to_string_lossy().to_string(),
        additional_paths: Vec::new(),
        sources: Vec::new(),
        verify_command: None,
        format_command: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// Insert a project rooted at `root` and a conversation bound to it.
async fn seed(storage: &Storage, root: &std::path::Path) -> String {
    seed_with_data(storage, &seeded_project_data(root)).await
}

/// Bind an arbitrary project payload to a fresh conversation.
async fn seed_with_data(storage: &Storage, data: &ProjectData) -> String {
    let project_id = Uuid::new_v4().to_string();
    let conversation_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&project_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(serde_json::to_string(data).expect("serialize"))
        .execute(storage.pool())
        .await
        .expect("insert project");
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&conversation_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(format!(r#"{{"projectId":"{project_id}"}}"#))
        .execute(storage.pool())
        .await
        .expect("insert conversation");
    conversation_id
}

/// A conversation with no project at all (its project was deleted).
async fn seed_orphan(storage: &Storage) -> String {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind("{}")
        .execute(storage.pool())
        .await
        .expect("insert conversation");
    id
}

#[tokio::test]
async fn agents_md_is_read_from_the_project_folder() {
    let storage = temp_storage().await;
    let root = temp_dir("agents");
    std::fs::write(root.join("AGENTS.md"), "always run `cargo test`").expect("write");
    let conversation_id = seed(&storage, &root).await;

    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::no_global(),
    )
    .await
    .expect("load");
    let system = runtime.system.expect("system block");
    assert!(system.contains("always run `cargo test`"));
    assert!(system.contains("AGENTS.md"));
    assert_eq!(runtime.root_path, Some(root.to_string_lossy().to_string()));
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn claude_md_is_used_when_there_is_no_agents_md() {
    let storage = temp_storage().await;
    let root = temp_dir("claude");
    std::fs::write(root.join("CLAUDE.md"), "prefer rustfmt defaults").expect("write");
    let conversation_id = seed(&storage, &root).await;

    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::no_global(),
    )
    .await
    .expect("load");
    let system = runtime.system.expect("system block");
    assert!(system.contains("prefer rustfmt defaults"));
    assert!(system.contains("CLAUDE.md"));
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn agents_md_wins_over_claude_md() {
    let storage = temp_storage().await;
    let root = temp_dir("both");
    std::fs::write(root.join("AGENTS.md"), "the standard file").expect("write");
    std::fs::write(root.join("CLAUDE.md"), "the other file").expect("write");
    let conversation_id = seed(&storage, &root).await;

    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::no_global(),
    )
    .await
    .expect("load");
    let system = runtime.system.expect("system block");
    assert!(system.contains("the standard file"));
    assert!(
        !system.contains("the other file"),
        "only one file must apply"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn the_global_file_applies_without_a_project() {
    let storage = temp_storage().await;
    let global_dir = temp_dir("global");
    std::fs::write(global_dir.join("AGENTS.md"), "company-wide rules").expect("write");
    let conversation_id = seed_orphan(&storage).await;

    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::with_codex_home(global_dir.clone()),
    )
    .await
    .expect("load");
    let system = runtime.system.expect("global block must apply on its own");
    assert!(system.contains("company-wide rules"));
    assert!(runtime.root_path.is_none());
    std::fs::remove_dir_all(&global_dir).ok();
}

#[tokio::test]
async fn a_conversation_without_instructions_stays_empty() {
    let storage = temp_storage().await;
    let conversation_id = seed_orphan(&storage).await;
    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::no_global(),
    )
    .await
    .expect("load");
    assert!(runtime.system.is_none());
}

#[tokio::test]
async fn instruction_files_are_capped() {
    let storage = temp_storage().await;
    let root = temp_dir("capped");
    // Far beyond the 32 KiB cap; the byte cap bounds the result.
    std::fs::write(root.join("AGENTS.md"), "x".repeat(400_000)).expect("write");
    let conversation_id = seed(&storage, &root).await;

    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::no_global(),
    )
    .await
    .expect("load");
    let system = runtime.system.expect("system block");
    assert!(system.len() < 400_000, "the file must be truncated");
    assert!(system.contains("truncated"), "truncation must be visible");
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_git_root_above_the_folder_widens_the_chain() {
    let storage = temp_storage().await;
    let root = temp_dir("nested");
    std::fs::create_dir_all(root.join(".git")).expect("mkdir git");
    let nested = root.join("services/payments");
    std::fs::create_dir_all(&nested).expect("mkdir");
    std::fs::write(root.join("AGENTS.md"), "root-level rules").expect("write");
    std::fs::write(nested.join("AGENTS.md"), "payments rules").expect("write");

    let mut data = seeded_project_data(&nested);
    data.name = "Payments".into();
    let conversation_id = seed_with_data(&storage, &data).await;

    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::no_global(),
    )
    .await
    .expect("load");
    let system = runtime.system.expect("system block");
    let root_pos = system.find("root-level rules").expect("git-root file");
    let leaf_pos = system.find("payments rules").expect("leaf file");
    assert!(
        root_pos < leaf_pos,
        "the git-root rules must precede the leaf rules"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn additional_paths_are_carried_into_the_runtime() {
    let storage = temp_storage().await;
    let root = temp_dir("extra-root");
    let extra = temp_dir("extra-lib");
    let mut data = seeded_project_data(&root);
    data.additional_paths = vec![extra.to_string_lossy().to_string()];
    let conversation_id = seed_with_data(&storage, &data).await;

    let runtime = load_project_runtime_with(
        storage.pool(),
        &conversation_id,
        &InstructionConfig::no_global(),
    )
    .await
    .expect("load");
    assert_eq!(
        runtime.additional_paths,
        vec![extra.to_string_lossy().to_string()]
    );
    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&extra).ok();
}
