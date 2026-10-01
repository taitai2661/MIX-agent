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
        auto_update: true,
        telemetry_consent: true,
        language: Some("ja".into()),
        theme: Some("dark".into()),
        projects_root: Some("/tmp/projects".into()),
        project_doc_max_bytes: Some(65_536),
        project_doc_fallback_filenames: vec!["TEAM_GUIDE.md".into()],
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
    assert!(!read.auto_update);
    assert_eq!(read.onboarding_completed, None);
}

/// Corrupted storage must not fail the read: the UI wants defaults, not an
/// error dialog, when the row is unreadable.
#[tokio::test]
async fn an_unreadable_row_falls_back_to_the_defaults() {
    let storage = temp_storage().await;
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, 'not json')",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .execute(storage.pool())
    .await
    .expect("insert garbage row");

    let read = read_settings(storage.pool()).await.expect("read");
    assert_eq!(read, SettingsView::default());
}
