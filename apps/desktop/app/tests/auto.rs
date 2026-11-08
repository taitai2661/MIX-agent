//! Auto resolution against a temporary `Storage`.

use mix_agent_desktop::commands::auto::{
    estimate_required_tokens, record_turn_performance, resolve_auto_model,
    resolve_auto_model_excluding,
};
use mix_agent_desktop::commands::settings::{write_settings, SettingsView};
use mixd_core::runs::engine::TurnPerformance;
use mixd_providers::reliability::ProviderErrorKind;

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
        0,
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
        0,
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
        0,
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
        0,
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
        0,
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
    let small = estimate_required_tokens(&["hi".to_string()], 100, 0);
    let large = estimate_required_tokens(&["hi".repeat(10_000)], 100, 0);
    assert!(large > small);
    let with_image = estimate_required_tokens(&["hi".to_string()], 100, 1_000);
    assert_eq!(with_image, small + 1_000);
}

#[tokio::test]
async fn measured_speed_moves_selection_when_priority_is_speed() {
    let storage = temp_storage("speed").await;
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
    // The models are otherwise identical: same provider, same capabilities,
    // same (unknown) quality prior. Only the measured speed differs.
    for (model, ttft, tps) in [
        ("p1:model-alpha", 5_000.0, 10.0),
        ("p1:model-beta", 500.0, 50.0),
    ] {
        sqlx::query(
            "INSERT INTO auto_model_stats \
               (model_id, profile, success_count, ewma_ttft_ms, ewma_output_tps) \
             VALUES (?, '*', 5, ?, ?)",
        )
        .bind(model)
        .bind(ttft)
        .bind(tps)
        .execute(storage.pool())
        .await
        .expect("insert stat");
    }
    write_settings(
        storage.pool(),
        SettingsView {
            auto_priority: "speed".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("write settings");

    let resolved = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        0,
        4096,
        "req-1",
        None,
    )
    .await
    .expect("resolve");
    assert_eq!(resolved.model_id, "p1:model-beta");
    assert!(
        resolved.selection.reason.contains("speed"),
        "{:?}",
        resolved.selection.reason
    );
}

#[tokio::test]
async fn auth_failures_cool_the_provider_down() {
    let storage = temp_storage("auth").await;
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
    let data = serde_json::json!({
        "modelId": "p1:model-alpha",
        "providerId": "p1",
        "scope": "chat",
        "outcome": "failure",
        "classification": "auth",
    });
    sqlx::query("INSERT INTO auto_reliability_events (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(storage.pool())
        .await
        .expect("insert auth event");

    let resolved = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        0,
        4096,
        "req-1",
        None,
    )
    .await
    .expect("resolve");
    assert_eq!(resolved.model_id, "p2:model-beta");
}

#[tokio::test]
async fn fallback_resolution_excludes_the_primary_and_prefers_another_provider() {
    let storage = temp_storage("fallback-chain").await;
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

    let primary = resolve_auto_model(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        0,
        4096,
        "req-1",
        None,
    )
    .await
    .expect("primary");
    let fallback = resolve_auto_model_excluding(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        0,
        4096,
        "req-1:fallback:0",
        None,
        std::slice::from_ref(&primary.model_id),
        Some(&primary.provider_id),
    )
    .await
    .expect("fallback");
    assert_ne!(fallback.model_id, primary.model_id);
    assert_ne!(fallback.provider_id, primary.provider_id);

    // With every candidate excluded, the chain ends instead of looping back to
    // the failed model.
    let err = resolve_auto_model_excluding(
        storage.pool(),
        "hello",
        "chat",
        false,
        false,
        &parts("hello").await,
        0,
        4096,
        "req-1:fallback:1",
        None,
        &[primary.model_id.clone(), fallback.model_id.clone()],
        Some(&primary.provider_id),
    )
    .await
    .unwrap_err();
    assert!(err.contains("Auto"), "got: {err}");
}

#[tokio::test]
async fn turn_performance_updates_stats_and_failure_events() {
    let storage = temp_storage("stats").await;
    let success = TurnPerformance {
        model_id: "p1:model-alpha".to_string(),
        provider_id: "p1".to_string(),
        ttft_ms: Some(500),
        duration_ms: 2_500,
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        failure_kind: None,
    };
    record_turn_performance(
        storage.pool(),
        "general:lang:en:task:general:diff:low",
        "chat",
        &success,
    )
    .await;

    let row: (i64, i64, Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT success_count, failure_count, ewma_ttft_ms, ewma_output_tps \
           FROM auto_model_stats WHERE model_id = ? AND profile = '*'",
    )
    .bind("p1:model-alpha")
    .fetch_one(storage.pool())
    .await
    .expect("stat row");
    assert_eq!(row.0, 1);
    assert_eq!(row.1, 0);
    assert!((row.2.unwrap() - 500.0).abs() < 1e-9);
    // 100 tokens over 2.0s of generation time.
    assert!((row.3.unwrap() - 50.0).abs() < 1e-9);

    let failure = TurnPerformance {
        model_id: "p1:model-alpha".to_string(),
        provider_id: "p1".to_string(),
        ttft_ms: None,
        duration_ms: 10,
        input_tokens: None,
        output_tokens: None,
        failure_kind: Some(ProviderErrorKind::Unavailable),
    };
    record_turn_performance(
        storage.pool(),
        "general:lang:en:task:general:diff:low",
        "chat",
        &failure,
    )
    .await;

    let row: (i64, i64, Option<f64>) = sqlx::query_as(
        "SELECT success_count, failure_count, ewma_ttft_ms \
           FROM auto_model_stats WHERE model_id = ? AND profile = '*'",
    )
    .bind("p1:model-alpha")
    .fetch_one(storage.pool())
    .await
    .expect("stat row");
    assert_eq!(row.0, 1);
    assert_eq!(row.1, 1);
    // A failed turn must not overwrite the successful speed sample.
    assert!((row.2.unwrap() - 500.0).abs() < 1e-9);

    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM auto_reliability_events")
        .fetch_one(storage.pool())
        .await
        .expect("event count");
    assert_eq!(events, 1, "one failure event per failed turn");
}
