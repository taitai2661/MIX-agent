//! Integration coverage for the model-table sync.
//!
//! `sync_models` is the only writer used by discovery, provider connect and the
//! local-runtime manager. Its list is authoritative: a model deleted from the
//! runtime must disappear from the table, not merely stop being returned by the
//! next probe.

use mix_agent_desktop::commands::models::sync_models;
use mixd_providers::discovery::DiscoveredModel;
use mixd_storage::Storage;

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-models-test-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

fn model(id: &str) -> DiscoveredModel {
    DiscoveredModel {
        id: id.to_string(),
        label: id.to_string(),
        ..DiscoveredModel::default()
    }
}

#[tokio::test]
async fn a_resync_prunes_models_the_runtime_no_longer_reports() {
    let storage = temp_storage().await;
    sync_models(
        storage.pool(),
        "provider-1",
        &[model("qwen3:8b"), model("llama3.2:3b")],
    )
    .await
    .expect("initial sync");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models")
        .fetch_one(storage.pool())
        .await
        .expect("count");
    assert_eq!(count, 2);

    // The user deleted `llama3.2:3b` from Ollama.
    let written = sync_models(storage.pool(), "provider-1", &[model("qwen3:8b")])
        .await
        .expect("resync");
    assert_eq!(written.len(), 1);

    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM models ORDER BY id")
        .fetch_all(storage.pool())
        .await
        .expect("read");
    assert_eq!(ids, vec!["provider-1:qwen3:8b".to_string()]);
}

#[tokio::test]
async fn a_resync_does_not_touch_another_providers_models() {
    let storage = temp_storage().await;
    sync_models(storage.pool(), "provider-1", &[model("a")])
        .await
        .expect("sync one");
    sync_models(storage.pool(), "provider-2", &[model("b")])
        .await
        .expect("sync two");

    // Emptying provider-1's runtime must not remove provider-2's row.
    sync_models(storage.pool(), "provider-1", &[])
        .await
        .expect("empty sync");

    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM models ORDER BY id")
        .fetch_all(storage.pool())
        .await
        .expect("read");
    assert_eq!(ids, vec!["provider-2:b".to_string()]);
}
