//! Integration coverage for the settings command layer.
//!
//! `app_settings` holds exactly one row (`id = 'singleton'`) and every save
//! replaces the stored document outright. The WebView relies on that: it sends
//! the whole form back, so a field sent as `null` has to clear what was there
//! rather than merge with it.

use mix_agent_desktop::commands::settings::{read_settings, write_settings, SettingsView};
use mixd_storage::Storage;

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-settings-test-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

fn full_settings() -> SettingsView {
    SettingsView {
        default_model_id: Some("prov:model-a".into()),
        default_mode: Some("agent".into()),
        egress_allowlist: vec!["https://api.example.com".into()],
        telemetry_consent: true,
        language: Some("ja".into()),
        theme: Some("dark".into()),
        projects_root: Some("/tmp/projects".into()),
        project_doc_max_bytes: Some(65_536),
        project_doc_fallback_filenames: vec!["TEAM_GUIDE.md".into()],
        verify_command: Some("cargo test --workspace".into()),
        format_command: Some("cargo fmt".into()),
        sandbox_terminal: true,
        memory_auto_formation: false,
        // Non-default on purpose: `true` is what proves the round trip carried
        // the key rather than re-deriving §4's `false`.
        memory_link_embedding_neighbors: true,
        database_encryption: true,
        onboarding_completed: Some(true),
    }
}

#[tokio::test]
async fn a_missing_row_reads_back_as_the_defaults() {
    let storage = temp_storage().await;
    let read = read_settings(storage.pool()).await.expect("read");
    assert_eq!(read, SettingsView::default());
    assert!(read.egress_allowlist.is_empty());
    assert_eq!(read.language, None);
    assert_eq!(read.onboarding_completed, None);
    // Codex parity: database encryption defaults off.
    assert!(
        !read.database_encryption,
        "database encryption must default off (Codex parity)"
    );
    // Memory spec §6: auto formation ships enabled; `false` is an opt-out.
    assert!(read.memory_auto_formation, "auto formation defaults to on");
    // Memory spec §4 「(3) は既定オフ」: the embedding-neighbour source ships
    // disabled, so `false` is the contract rather than a derived accident.
    assert!(
        !read.memory_link_embedding_neighbors,
        "embedding-neighbour links must default off (spec §4)"
    );
}

/// A row written before the onboarding flag existed has no such key. Serde must
/// fill it in rather than failing the whole document, otherwise upgrading the
/// app would hand every existing user the wizard again — which is also the
/// behaviour a fresh install depends on.
#[tokio::test]
async fn a_row_without_the_onboarding_key_reads_back_as_never_onboarded() {
    let storage = temp_storage().await;
    sqlx::query("INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?)")
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(r#"{"default_mode":"agent","theme":"dark"}"#)
        .execute(storage.pool())
        .await
        .expect("insert legacy row");

    let read = read_settings(storage.pool()).await.expect("read");
    assert_eq!(read.onboarding_completed, None);
    // The rest of the legacy document still deserializes.
    assert_eq!(read.default_mode.as_deref(), Some("agent"));
    assert_eq!(read.theme.as_deref(), Some("dark"));
    // A document that predates the auto-formation key reads as enabled.
    assert!(
        read.memory_auto_formation,
        "a legacy row must not silently disable memory auto formation"
    );
    // The opposite contract: a document that predates the §4 source-(3) key
    // must read as *off*, because §4 says the source ships off.
    assert!(
        !read.memory_link_embedding_neighbors,
        "a legacy row must not silently enable embedding-neighbour links"
    );
    assert!(
        !read.database_encryption,
        "a legacy row must not silently enable database encryption"
    );
}

#[tokio::test]
async fn a_full_row_round_trips() {
    let storage = temp_storage().await;
    let expected = full_settings();
    write_settings(storage.pool(), expected.clone())
        .await
        .expect("write");

    let read = read_settings(storage.pool()).await.expect("read");
    assert_eq!(read, expected);
}

/// The `ON CONFLICT(id) DO UPDATE SET data = excluded.data` clause has to
/// replace the whole document: two saves must leave one row that equals the
/// *second* value, with every field of the first one gone.
#[tokio::test]
async fn the_second_save_replaces_every_field_and_stays_a_single_row() {
    let storage = temp_storage().await;
    let pool = storage.pool();

    write_settings(pool, full_settings())
        .await
        .expect("first write");

    let cleared = SettingsView::default();
    write_settings(pool, cleared.clone())
        .await
        .expect("second write");

    let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM app_settings")
        .fetch_all(pool)
        .await
        .expect("list rows");
    assert_eq!(ids.len(), 1, "settings must stay a single row");
    assert_eq!(ids[0].0, "singleton");

    let read = read_settings(pool).await.expect("read");
    assert_eq!(read, cleared, "the first write must not bleed through");
    assert_eq!(read.default_model_id, None);
    assert_eq!(read.language, None);
    assert_eq!(read.theme, None);
    assert!(read.egress_allowlist.is_empty());
    assert_eq!(read.onboarding_completed, None);
}

/// A corrupt row must surface an error — not silently reset every preference
/// to the defaults — so the caller can preserve the row instead of blanking it.
#[tokio::test]
async fn an_unreadable_row_returns_an_error_and_preserves_the_row() {
    let storage = temp_storage().await;
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, 'not json')",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .execute(storage.pool())
    .await
    .expect("insert garbage row");

    let err = read_settings(storage.pool())
        .await
        .expect_err("a corrupt row must not read as defaults");
    assert!(err.contains("corrupt"), "unexpected error: {err}");

    // The row is untouched: nothing was overwritten with defaults.
    let raw: (String,) = sqlx::query_as("SELECT data FROM app_settings WHERE id = 'singleton'")
        .fetch_one(storage.pool())
        .await
        .expect("row preserved");
    assert_eq!(raw.0, "not json");
}
