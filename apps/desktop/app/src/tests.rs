use crate::state::load_egress_policy;
use crate::{worktree, APP_IDENTIFIER};

/// The untrusted browser child WebView must not inherit the main
/// WebView's IPC capability. A `windows = ["main"]` grant applies to every
/// webview in that window, so the capability must be scoped by webview.
#[test]
fn browser_capability_is_scoped_to_the_main_webview() {
    let capability = include_str!("../capabilities/default.toml");
    assert!(capability.contains("webviews = [\"main\"]"));
    assert!(
        !capability
            .lines()
            .any(|line| line.trim_start().starts_with("windows =")),
        "a window-scoped capability would expose IPC to the browser WebView"
    );
}

/// The log directory is derived from the bundle identifier without a Tauri
/// handle, so the constant has to stay in sync with the config file by
/// hand — this is what catches the drift. The Keychain service name is the
/// third party: it aliases the same string, so all three are asserted.
#[test]
fn app_identifier_matches_the_tauri_config() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let raw = std::fs::read_to_string(&path).expect("read tauri.conf.json");
    let config: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");
    let identifier = config
        .get("identifier")
        .and_then(|v| v.as_str())
        .expect("tauri.conf.json has an identifier");
    assert_eq!(
        identifier, APP_IDENTIFIER,
        "APP_IDENTIFIER drifted from tauri.conf.json"
    );
    assert_eq!(
        identifier,
        mixd_storage::SERVICE,
        "Keychain SERVICE drifted from tauri.conf.json"
    );
}

/// Apple Silicon refuses to launch a completely unsigned `.app` copied out
/// of a downloaded DMG and reports it as damaged. Keep the ad-hoc signing
/// identity in the config so a release build can never regress to unsigned
/// again; Developer ID signing/notarization remains opt-in.
#[test]
fn macos_bundle_keeps_the_ad_hoc_signing_identity() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let raw = std::fs::read_to_string(&path).expect("read tauri.conf.json");
    let config: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");
    let identity = config
        .pointer("/bundle/macOS/signingIdentity")
        .and_then(|v| v.as_str());
    assert_eq!(
        identity,
        Some("-"),
        "bundle.macOS.signingIdentity must stay \"-\" (ad-hoc) so macOS can launch the app"
    );
}

/// Extract `version = "x.y.z"` from exactly one TOML section.
///
/// A naive scan for the first `version` line after `[workspace.package]`
/// never notices the next section header, so a `version` key in a later
/// section (or a commented-out line) could be picked up silently. This
/// parser tracks the current section, stops at the next header, skips
/// comments/blank lines, and requires exactly one quoted `version` value
/// inside the section.
fn toml_section_version(toml: &str, section: &str) -> String {
    let mut in_section = false;
    let mut found: Option<String> = None;
    for raw_line in toml.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // A header ends at the first `]`; anything after it (trailing
            // comment) is ignored for section tracking.
            let header = match line.find(']') {
                Some(idx) => line[..=idx].trim(),
                None => line,
            };
            in_section = header == section;
            continue;
        }
        if !in_section {
            continue;
        }
        // Strip a trailing comment (`version = "0.1.0" # comment`).
        let code = match line.find('#') {
            Some(idx) => line[..idx].trim_end(),
            None => line,
        };
        let Some(rest) = code.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let value = rest.trim();
        assert!(
            value.len() >= 2 && value.starts_with('"') && value.ends_with('"'),
            "version in {section} must be a quoted string, got: {value}"
        );
        assert!(found.is_none(), "duplicate version key in {section}");
        found = Some(value[1..value.len() - 1].to_string());
    }
    found.unwrap_or_else(|| panic!("no version key in {section}"))
}

/// `version` is declared in 4 places (workspace Cargo.toml, app Cargo via
/// workspace, tauri.conf.json, package.json + ui/package.json) and they
/// must ship the same number — this is what catches the drift.
#[test]
fn app_version_matches_all_manifests() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_toml = std::fs::read_to_string(manifest_dir.join("..").join("Cargo.toml"))
        .expect("read workspace Cargo.toml");
    let cargo_version = toml_section_version(&workspace_toml, "[workspace.package]");
    let tauri_raw = std::fs::read_to_string(manifest_dir.join("tauri.conf.json"))
        .expect("read tauri.conf.json");
    let tauri: serde_json::Value = serde_json::from_str(&tauri_raw).expect("parse tauri.conf.json");
    let tauri_version = tauri
        .get("version")
        .and_then(|v| v.as_str())
        .expect("tauri version");
    assert_eq!(
        env!("CARGO_PKG_VERSION"),
        cargo_version.as_str(),
        "CARGO_PKG_VERSION drifted from workspace Cargo.toml"
    );
    assert_eq!(
        tauri_version,
        cargo_version.as_str(),
        "tauri.conf.json version drifted from workspace Cargo.toml"
    );
    for rel in ["../package.json", "../ui/package.json"] {
        let raw = std::fs::read_to_string(manifest_dir.join(rel)).expect("read package.json");
        let pkg: serde_json::Value = serde_json::from_str(&raw).expect("parse package.json");
        let v = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .expect("pkg version");
        assert_eq!(v, cargo_version.as_str(), "version drift in {rel}");
    }
}

async fn temp_storage() -> mixd_storage::Storage {
    let dir = std::env::temp_dir().join(format!("mixd-egress-test-{}", uuid::Uuid::new_v4()));
    mixd_storage::Storage::open(&dir)
        .await
        .expect("open temp storage")
}

/// The allowlist entered in Settings → Network has to survive the round trip
/// into the policy the tools actually enforce. It is stored snake_case
/// (`SettingsView` has no `rename_all`), and the loader once looked for
/// `egressAllowlist` instead — so every configured allowlist was silently
/// dropped and `web_fetch` ran wide-open to every public domain.
#[tokio::test]
async fn the_configured_egress_allowlist_reaches_the_policy() {
    let storage = temp_storage().await;
    sqlx::query("INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?)")
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(r#"{"egress_allowlist":["api.example.com","*.internal.test"]}"#)
        .execute(storage.pool())
        .await
        .expect("insert settings row");

    let policy = load_egress_policy(&storage).await;
    assert_eq!(policy.allowlist, ["api.example.com", "*.internal.test"]);
    assert!(policy.enforce("https://api.example.com/v1").is_ok());
    assert!(policy.enforce("https://elsewhere.test/v1").is_err());
}

/// The wide-open default is a documented behaviour, not an accident: an
/// empty allowlist allows public hosts and still refuses private IPs.
#[tokio::test]
async fn an_unconfigured_allowlist_stays_wide_open_for_public_hosts() {
    let storage = temp_storage().await;
    let policy = load_egress_policy(&storage).await;
    assert!(policy.allowlist.is_empty());
    assert!(policy.enforce("https://example.com/").is_ok());
    assert!(policy.enforce("https://127.0.0.1/").is_err());
}

/// A row written by an older build must not fail the launch: the tools fall
/// back to the defaults rather than the app refusing to start.
#[tokio::test]
async fn an_unreadable_settings_row_still_yields_a_usable_policy() {
    let storage = temp_storage().await;
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, 'not json')",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .execute(storage.pool())
    .await
    .expect("insert garbage row");

    let policy = load_egress_policy(&storage).await;
    assert!(policy.allowlist.is_empty());
}

/// A delegated worktree must see uncommitted *and* untracked parent work,
/// without the parent's real index being modified.
#[tokio::test]
async fn a_delegate_base_includes_untracked_files() {
    use std::process::Command;

    let root = std::env::temp_dir().join(format!("mixd-delegate-base-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create repo");
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .current_dir(&root)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    std::fs::write(root.join("tracked.txt"), "one").expect("write");
    git(&["add", "tracked.txt"]);
    git(&["commit", "-qm", "initial"]);
    std::fs::write(root.join("tracked.txt"), "two").expect("write");
    std::fs::write(root.join("untracked.txt"), "new").expect("write");
    let head = Command::new("git")
        .current_dir(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse");
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();

    let base = worktree::delegate_base_commit(&root, &head)
        .await
        .expect("delegate base");
    assert_ne!(base, head, "a dirty worktree must get a new base commit");
    let tree = Command::new("git")
        .current_dir(&root)
        .args(["ls-tree", "-r", "--name-only", &base])
        .output()
        .expect("git ls-tree");
    let tree = String::from_utf8_lossy(&tree.stdout);
    assert!(tree.contains("tracked.txt"));
    assert!(tree.contains("untracked.txt"));

    // The parent index still considers the new file untracked.
    let status = Command::new("git")
        .current_dir(&root)
        .args(["status", "--porcelain"])
        .output()
        .expect("git status");
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(status.contains("untracked.txt"));

    let _ = std::fs::remove_dir_all(&root);
}
