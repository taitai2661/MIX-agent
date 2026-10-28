//! Chromium-free coverage for the Browser Operator's configuration and scope
//! rules: profile directories, the Tool spec the model is shown, and the
//! agreement between the selected backend and the executable it would launch.
//!
//! Nothing here starts a browser — launch failures are logged by the backend
//! itself and need a real Chromium to provoke.

use mixd_browser::profile::{default_profile_root, BrowserProfile};
use mixd_browser::{
    browser_tool_spec, desktop_agent_actions, BrowserAction, BrowserBackend, BrowserOperator,
    WaitCondition,
};

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("mixd-browser-scope-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn profiles_live_under_the_app_data_directory() {
    let root = temp_root("root");
    let profile_root = default_profile_root(&root);
    assert_eq!(profile_root, root.join("browser"));
    let profile = BrowserProfile::new(uuid::Uuid::new_v4(), "demo", &profile_root);
    assert!(
        profile
            .directory
            .starts_with(root.to_string_lossy().as_ref()),
        "profiles must not be written into the repository: {}",
        profile.directory
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn a_profile_is_created_on_demand_and_wiped_by_reset() {
    let root = temp_root("lifecycle");
    let id = uuid::Uuid::new_v4();
    let mut profile = BrowserProfile::new(id, "workspace", &root);
    assert!(!profile.provisioned);
    assert!(!profile.path().exists());

    profile.ensure().await.expect("ensure");
    assert!(profile.provisioned);
    assert!(profile.path().is_dir());

    // Re-running against a directory that already exists must succeed too.
    profile.ensure().await.expect("second ensure");

    profile.reset().await.expect("reset");
    assert!(!profile.path().exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_tool_spec_only_requires_an_action() {
    let spec = browser_tool_spec();
    assert_eq!(spec["name"], "browser");
    assert_eq!(
        spec["parameters"]["required"],
        serde_json::json!(["action"])
    );
    assert_eq!(spec["parameters"]["type"], serde_json::json!("object"));

    let advertised: Vec<String> = spec["parameters"]["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert_eq!(
        advertised,
        desktop_agent_actions(),
        "the tool spec and the model-facing contract disagree"
    );
    // The manual surface still supports these; only the model-facing enum
    // hides them.
    for manual in ["click_at", "type_active"] {
        assert!(
            !advertised.iter().any(|action| action == manual),
            "{manual} is manual-only and must not be advertised to the model"
        );
    }
}

/// The spec's `action` enum is a hand-written string list while the enum is
/// derived from the Rust type, so they can drift apart: a new action would be
/// callable but invisible to the model (or vice versa).
#[test]
fn every_action_serialises_to_a_kind_the_spec_advertises() {
    let spec = browser_tool_spec();
    let advertised: Vec<String> = spec["parameters"]["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    let actions = [
        BrowserAction::Navigate {
            url: "https://example.com".into(),
        },
        BrowserAction::Click {
            selector: "#go".into(),
            reference: None,
        },
        BrowserAction::Type {
            selector: "#q".into(),
            text: "hi".into(),
            reference: None,
            clear: false,
        },
        BrowserAction::ClickAt { x: 10, y: 20 },
        BrowserAction::TypeActive { text: "hi".into() },
        BrowserAction::Press {
            key: "Enter".into(),
        },
        BrowserAction::Hover {
            selector: "#menu".into(),
            reference: None,
        },
        BrowserAction::Select {
            selector: "#choice".into(),
            value: "a".into(),
            reference: None,
        },
        BrowserAction::Check {
            selector: "#agree".into(),
            checked: true,
            reference: None,
        },
        BrowserAction::Scroll { x: 0, y: 100 },
        BrowserAction::Dom { selector: None },
        BrowserAction::Extract {
            selector: "a".into(),
            limit: 20,
        },
        BrowserAction::Snapshot { max_nodes: 50 },
        BrowserAction::Screenshot { full_page: false },
        BrowserAction::Evaluate {
            script: "1 + 1".into(),
        },
        BrowserAction::WaitFor {
            condition: WaitCondition::Selector {
                selector: "#ready".into(),
            },
            timeout_ms: 1_000,
        },
        BrowserAction::Info,
        BrowserAction::Reload,
        BrowserAction::Back,
        BrowserAction::Forward,
        BrowserAction::Tabs,
        BrowserAction::NewTab { url: None },
        BrowserAction::SwitchTab {
            tab_id: "tab".into(),
        },
        BrowserAction::CloseTab { tab_id: None },
        BrowserAction::Close,
    ];
    for action in actions {
        let value = serde_json::to_value(&action).expect("serialise");
        let kind = value["kind"].as_str().expect("tagged kind").to_string();
        if kind == "click_at" || kind == "type_active" {
            assert!(
                !advertised.contains(&kind),
                "manual-only {kind} must stay out of the model-facing enum"
            );
        } else {
            assert!(
                advertised.contains(&kind),
                "{kind} is not in the spec enum {advertised:?}"
            );
        }
    }
}

/// `resolve_executable` decides the backend; the two halves have to agree or
/// the operator would report a working browser that cannot be launched.
#[test]
fn the_selected_backend_agrees_with_the_executable_it_would_use() {
    let dir = temp_root("resolve");
    let operator = BrowserOperator::new(&dir, true);
    let (backend, executable) = operator.resolve_executable();
    match backend {
        BrowserBackend::Disabled => assert!(
            executable.is_none(),
            "a disabled backend must not claim an executable"
        ),
        BrowserBackend::Chromium | BrowserBackend::SystemChromium => assert!(
            executable.is_some(),
            "{backend:?} was selected without a path to launch"
        ),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
