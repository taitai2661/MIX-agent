//! Integration coverage for the provider registry and the model rows hanging
//! off it.
//!
//! Two failures are pinned here, both of which only showed up *after* a correct
//! provider had been configured:
//!
//!  - A provider added during the session never reached the live
//!    `ProviderRegistry`, which is the only thing `RunEngine::execute` consults.
//!    `resolve_provider_for_model` therefore fell through to an empty list and
//!    every message failed with `no provider configured`.
//!  - Deleting a provider removed its `providers` row but not its `models` rows,
//!    so the Composer kept offering models that no provider could serve.
//!
//! The Keychain is not written to by anything here: the adapters read their key
//! through `SecretStore::get`, which reports `NoEntry` for a slot that was never
//! stored, and that is the same "no key configured" state these tests want.

use mix_agent_desktop::commands::models::{delete_models_for_provider, fetch_models};
use mix_agent_desktop::{register_adapter, resolve_provider_for_model};
use mixd_core::runs::engine::ProviderRegistry;
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::{SecretStore, Storage};
use uuid::Uuid;

async fn temp_storage(tag: &str) -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-providers-test-{tag}-{}", Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

async fn insert_provider_row(storage: &Storage, id: &str, enabled: bool) {
    sqlx::query("INSERT INTO providers (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(id)
        .bind(SINGLE_USER_ID)
        .bind(
            serde_json::json!({
                "kind": "compatible",
                "label": "Test",
                "baseUrl": "http://127.0.0.1:9/v1",
                "enabled": enabled,
                "preset": null,
                "extra": {},
            })
            .to_string(),
        )
        .execute(storage.pool())
        .await
        .expect("insert provider row");
}

async fn insert_model_row(storage: &Storage, model_id: &str, provider_id: &str) {
    sqlx::query("INSERT INTO models (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(model_id)
        .bind(SINGLE_USER_ID)
        .bind(
            serde_json::json!({
                "providerId": provider_id,
                "label": model_id,
                "contextWindow": 8192,
                "vision": false,
                "toolCalling": false,
                "reasoning": false,
                "auto": true,
            })
            .to_string(),
        )
        .execute(storage.pool())
        .await
        .expect("insert model row");
}

/// The behaviour `insert_provider` relies on: registering an adapter is what
/// makes a model id's `provider:` prefix resolvable.
#[tokio::test]
async fn registering_an_adapter_makes_the_models_prefix_resolvable() {
    let storage = temp_storage("register").await;
    let secrets = SecretStore::new();
    let registry = ProviderRegistry::default();
    let id = Uuid::new_v4().to_string();

    register_adapter(
        &registry,
        &secrets,
        &id,
        "compatible",
        "http://127.0.0.1:9/v1",
        true,
    )
    .await;
    assert!(
        registry.get(&id).await.is_some(),
        "the adapter must be reachable by id"
    );

    let resolved =
        resolve_provider_for_model(&registry, &storage, &secrets, &format!("{id}:gpt-4o"))
            .await
            .expect("resolve");
    assert_eq!(resolved, id);
}

/// The reported failure: a provider that exists in the table but was never put
/// in the registry. Resolution has to read the row back and build the adapter on
/// the spot rather than giving up.
#[tokio::test]
async fn a_stored_provider_the_registry_never_saw_is_still_resolvable() {
    let storage = temp_storage("lazy").await;
    let secrets = SecretStore::new();
    let registry = ProviderRegistry::default();
    let id = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &id, true).await;

    assert!(
        registry.get(&id).await.is_none(),
        "precondition: the registry starts empty"
    );

    let resolved =
        resolve_provider_for_model(&registry, &storage, &secrets, &format!("{id}:gpt-4o"))
            .await
            .expect("a stored, enabled provider must resolve");
    assert_eq!(resolved, id);
    assert!(
        registry.get(&id).await.is_some(),
        "resolution must leave the adapter in the registry for the next turn"
    );
}

/// A disabled provider is stored but unreachable, and hydrating it would defeat
/// the point of disabling it.
#[tokio::test]
async fn a_disabled_provider_is_neither_registered_nor_hydrated() {
    let storage = temp_storage("disabled").await;
    let secrets = SecretStore::new();
    let registry = ProviderRegistry::default();
    let id = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &id, false).await;

    register_adapter(
        &registry,
        &secrets,
        &id,
        "compatible",
        "http://127.0.0.1:9/v1",
        false,
    )
    .await;
    assert!(registry.get(&id).await.is_none());

    let err = resolve_provider_for_model(&registry, &storage, &secrets, &format!("{id}:gpt-4o"))
        .await
        .expect_err("no provider is registered at all");
    assert!(
        err.to_string().contains("no provider configured")
            || err.to_string().contains("provider not found"),
        "unexpected error: {err}"
    );
}

/// Disabling a provider has to remove the adapter that is already live; only
/// flagging the row left it serving for the rest of the session.
#[tokio::test]
async fn disabling_takes_the_live_adapter_out_of_the_registry() {
    let storage = temp_storage("unregister").await;
    let secrets = SecretStore::new();
    let registry = ProviderRegistry::default();
    let id = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &id, true).await;

    register_adapter(
        &registry,
        &secrets,
        &id,
        "compatible",
        "http://127.0.0.1:9/v1",
        true,
    )
    .await;
    assert!(registry.get(&id).await.is_some());

    register_adapter(
        &registry,
        &secrets,
        &id,
        "compatible",
        "http://127.0.0.1:9/v1",
        false,
    )
    .await;
    assert!(
        registry.get(&id).await.is_none(),
        "a disabled provider must not stay reachable"
    );
}

/// A bare model name (a hand-typed id, or one written before discovery) has no
/// prefix to read, so the registry is the only source. With nothing in it the
/// message has to say what to do rather than report an internal state.
#[tokio::test]
async fn an_empty_registry_reports_how_to_configure_a_provider() {
    let storage = temp_storage("empty").await;
    let secrets = SecretStore::new();
    let registry = ProviderRegistry::default();

    let err = resolve_provider_for_model(&registry, &storage, &secrets, "gpt-4o")
        .await
        .expect_err("nothing to resolve");
    let message = err.to_string();
    assert!(
        message.contains("no provider configured"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("プロバイダー"),
        "the message should name the setting to change: {message}"
    );
}

/// A bare model name still works when something *is* registered: the first
/// handle is the documented degraded fallback.
#[tokio::test]
async fn a_bare_model_id_falls_back_to_a_registered_provider() {
    let storage = temp_storage("fallback").await;
    let secrets = SecretStore::new();
    let registry = ProviderRegistry::default();
    let id = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &id, true).await;
    register_adapter(
        &registry,
        &secrets,
        &id,
        "compatible",
        "http://127.0.0.1:9/v1",
        true,
    )
    .await;

    let resolved = resolve_provider_for_model(&registry, &storage, &secrets, "gpt-4o")
        .await
        .expect("fallback");
    assert_eq!(resolved, id);
}

#[tokio::test]
async fn deleting_a_provider_removes_its_models_and_leaves_the_others() {
    let storage = temp_storage("delete").await;
    let kept = Uuid::new_v4().to_string();
    let gone = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &kept, true).await;
    insert_provider_row(&storage, &gone, true).await;
    insert_model_row(&storage, &format!("{kept}:gpt-4o"), &kept).await;
    insert_model_row(&storage, &format!("{kept}:gpt-4o-mini"), &kept).await;
    insert_model_row(&storage, &format!("{gone}:llama3.1:8b"), &gone).await;

    let mut tx = storage.pool().begin().await.expect("begin");
    delete_models_for_provider(&mut *tx, &gone)
        .await
        .expect("delete models");
    sqlx::query("DELETE FROM providers WHERE id = ?")
        .bind(&gone)
        .execute(&mut *tx)
        .await
        .expect("delete provider");
    tx.commit().await.expect("commit");

    let listed = fetch_models(storage.pool()).await.expect("list models");
    // Compared as a set: `created_at` only has millisecond resolution, so two
    // rows written in the same tick tie and the `ORDER BY` between them is not
    // defined. What matters here is which rows survived.
    let mut ids: Vec<String> = listed.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    let mut expected = vec![format!("{kept}:gpt-4o"), format!("{kept}:gpt-4o-mini")];
    expected.sort();
    assert_eq!(ids, expected, "only the deleted provider's models may go");
}

/// The delete cascade only helps rows written *after* it shipped, so the read
/// path has to defend itself: a model whose provider row is gone must never
/// reach the Composer, however it got there.
#[tokio::test]
async fn an_orphaned_model_is_not_listed() {
    let storage = temp_storage("orphan").await;
    let live = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &live, true).await;
    insert_model_row(&storage, &format!("{live}:gpt-4o"), &live).await;
    // As if a provider had been deleted by an older build: the model row
    // survives, its provider does not.
    insert_model_row(
        &storage,
        &format!("{}-gone:llama3.1:8b", Uuid::new_v4()),
        &Uuid::new_v4().to_string(),
    )
    .await;

    let listed = fetch_models(storage.pool()).await.expect("list models");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].provider_id, live);
}

/// A row that never got a `providerId` — only reachable from a hand-edited or
/// partially restored database — used to make `query_as` fail on the NULL and
/// blank the whole list. One bad row must cost one row.
#[tokio::test]
async fn a_malformed_row_does_not_take_the_whole_list_down() {
    let storage = temp_storage("malformed").await;
    let live = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &live, true).await;
    insert_model_row(&storage, &format!("{live}:gpt-4o"), &live).await;
    sqlx::query("INSERT INTO models (id, owner_id, data) VALUES (?, ?, ?)")
        .bind("hand-written")
        .bind(SINGLE_USER_ID)
        .bind(serde_json::json!({ "label": "no providerId" }).to_string())
        .execute(storage.pool())
        .await
        .expect("insert malformed row");

    let listed = fetch_models(storage.pool())
        .await
        .expect("a malformed row must not fail the query");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, format!("{live}:gpt-4o"));
}

/// Model ids are `<provider_id>:<model>`, and a local-runtime tag carries its own
/// colon. `delete_models_for_provider` matches on `data.$.providerId` rather than
/// on the id prefix precisely so that tag survives intact — this pins both the
/// matching and the intact id.
#[tokio::test]
async fn a_tagged_local_model_is_stored_and_removed_as_whole() {
    let storage = temp_storage("tags").await;
    let id = Uuid::new_v4().to_string();
    insert_provider_row(&storage, &id, true).await;
    insert_model_row(&storage, &format!("{id}:llama3.1:8b"), &id).await;

    let listed = fetch_models(storage.pool()).await.expect("list models");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, format!("{id}:llama3.1:8b"));

    delete_models_for_provider(storage.pool(), &id)
        .await
        .expect("delete");
    assert!(fetch_models(storage.pool()).await.expect("list").is_empty());
}
