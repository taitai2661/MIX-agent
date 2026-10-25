//! Auto resolution against a temporary `Storage`.

use mix_agent_desktop::commands::auto::{estimate_required_tokens, resolve_auto_model};

async fn temp_storage(name: &str) -> mixd_storage::Storage {
    let dir =
        std::env::temp_dir().join(format!("mixd-auto-test-{}-{}", name, uuid::Uuid::new_v4()));
    mixd_storage::Storage::open(&dir)
        .await
        .expect("open temp storage")
}

async fn insert_provider(storage: &mixd_storage::Storage, id: &str) {
    sqlx::query("INSERT INTO providers (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(r#"{"kind":"openai","label":"Test","enabled":true}"#)
        .execute(storage.pool())
        .await
        .expect("insert provider");
}

#[allow(clippy::too_many_arguments)]
async fn insert_model(
    storage: &mixd_storage::Storage,
    id: &str,
    provider_id: &str,
    label: &str,
    context_window: u32,
    vision: bool,
    tool_calling: bool,
    auto: bool,
) {
    let data = serde_json::json!({
        "providerId": provider_id,
        "label": label,
        "contextWindow": context_window,
        "vision": vision,
        "toolCalling": tool_calling,
        "reasoning": false,
        "auto": auto,
    });
    sqlx::query("INSERT INTO models (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(storage.pool())
        .await
        .expect("insert model");
}

async fn parts(content: &str) -> Vec<String> {
    vec![content.to_string()]
}

#[tokio::test]
async fn resolves_to_an_eligible_candidate() {
    let storage = temp_storage("basic").await;
    insert_provider(&storage, "p1").await;
    insert_model(
        &storage,
        "p1:gpt-4o-mini",
        "p1",
        "mini",
        128_000,
        false,
        true,
        true,
    )
    .await;
    insert_model(
        &storage,
        "p1:gpt-4o",
        "p1",
        "full",
        128_000,
        true,
        true,
        true,
    )
    .await;

    let resolved = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        true,
        &parts("hello").await,
        4096,
        "req-1",
        None,
    )
    .await
    .expect("resolve");
    // Only the vision model can serve an image turn.
    assert_eq!(resolved.model_id, "p1:gpt-4o");
    assert_eq!(resolved.provider_id, "p1");
    assert_eq!(resolved.selection.candidate_count, 1);
}

#[tokio::test]
async fn errors_when_no_candidate_can_serve() {
    let storage = temp_storage("empty").await;
    insert_provider(&storage, "p1").await;
    insert_model(
        &storage, "p1:tiny", "p1", "tiny", 128_000, false, true, true,
    )
    .await;

    let err = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        true,
        &parts("hello").await,
        4096,
        "req-1",
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("Vision"), "got: {err}");
}

#[tokio::test]
async fn errors_when_nothing_is_opted_in() {
    let storage = temp_storage("optout").await;
    insert_provider(&storage, "p1").await;
    insert_model(
        &storage,
        "p1:gpt-4o",
        "p1",
        "full",
        128_000,
        true,
        true,
        false,
    )
    .await;

    let err = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        4096,
        "req-1",
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("Auto"), "got: {err}");
}

#[tokio::test]
async fn learned_feedback_moves_future_selections() {
    let storage = temp_storage("feedback").await;
    insert_provider(&storage, "p1").await;
    insert_model(
        &storage,
        "p1:model-alpha",
        "p1",
        "a",
        128_000,
        false,
        false,
        true,
    )
    .await;
    insert_model(
        &storage,
        "p1:model-beta",
        "p1",
        "b",
        128_000,
        false,
        false,
        true,
    )
    .await;

    // Down-vote alpha on this exact profile several times; beta should win.
    let profile = mixd_core::auto::routing_profile("hello", "chat", false, false);
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, '{}')")
        .bind("conv-1")
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .execute(storage.pool())
        .await
        .expect("insert conversation");
    for _ in 0..5 {
        let message_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id, owner_id, data, conversation_id) VALUES (?, ?, '{}', 'conv-1')",
        )
        .bind(&message_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .execute(storage.pool())
        .await
        .expect("insert message");
        let data = serde_json::json!({
            "modelId": "p1:model-alpha",
            "profile": profile,
            "value": "down",
        });
        sqlx::query(
            "INSERT INTO message_feedback (id, owner_id, data, message_id) VALUES (?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(&message_id)
        .execute(storage.pool())
        .await
        .expect("insert feedback");
    }
    let resolved = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        4096,
        "req-1",
        None,
    )
    .await
    .expect("resolve");
    assert_eq!(resolved.model_id, "p1:model-beta");
}

#[tokio::test]
async fn recent_failures_penalize_the_model() {
    let storage = temp_storage("reliability").await;
    insert_provider(&storage, "p1").await;
    insert_provider(&storage, "p2").await;
    insert_model(
        &storage,
        "p1:model-alpha",
        "p1",
        "a",
        128_000,
        false,
        false,
        true,
    )
    .await;
    insert_model(
        &storage,
        "p2:model-beta",
        "p2",
        "b",
        128_000,
        false,
        false,
        true,
    )
    .await;

    for _ in 0..2 {
        let data = serde_json::json!({
            "modelId": "p1:model-alpha",
            "providerId": "p1",
            "scope": "chat",
            "outcome": "failure",
            "classification": "provider_5xx",
        });
        sqlx::query("INSERT INTO auto_reliability_events (id, owner_id, data) VALUES (?, ?, ?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(data.to_string())
            .execute(storage.pool())
            .await
            .expect("insert reliability event");
    }
    let resolved = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        4096,
        "req-1",
        None,
    )
    .await
    .expect("resolve");
    assert_eq!(resolved.model_id, "p2:model-beta");
}

#[test]
fn token_estimate_grows_with_content() {
    let small = estimate_required_tokens(&["hi".to_string()], 100);
    let large = estimate_required_tokens(&["hi".repeat(10_000)], 100);
    assert!(large > small);
}
