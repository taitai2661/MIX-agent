//! Real-Chromium integration coverage.
//!
//! These tests are skipped (not failed) when no Chromium is available. Set
//! `MIX_REQUIRE_BROWSER_TEST=1` to make a missing browser a hard failure in a
//! CI job that is expected to provide one.
//!
//! The fixture is a local HTTP server, so the test also exercises the explicit
//! local-network opt-in instead of depending on the public internet.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

use mixd_browser::{
    find_bundled_chromium, BrowserAction, BrowserCall, BrowserOperator, BrowserTool, WaitCondition,
};

/// Chromium screenshot capture is not reliable with several browser instances
/// running at once on a headless macOS runner. Serialise the real-browser
/// tests; they are fast (a few seconds) and this removes a class of flakes
/// without weakening what they cover.
static SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn browser_version(executable: &Path) -> Option<String> {
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().last().map(str::to_string)
}

fn chromium() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MIX_BROWSER_EXECUTABLE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pinned = mixd_browser::chromium_manifest().version.clone();
    for root in [
        manifest.join("../app/resources"),
        manifest.join("../../target/debug"),
    ] {
        if let Some(path) = find_bundled_chromium(Some(&root)) {
            // A legacy bundle can outlive a version bump (`app/resources` is
            // gitignored). Screenshot behaviour is version-sensitive, so a
            // stale 145 tree must skip rather than turn a pinned-156 regression
            // into a red local suite.
            match browser_version(&path) {
                Some(version) if version != pinned => {
                    eprintln!(
                        "skipping browser integration test: bundled {version} != pinned {pinned}"
                    );
                    return None;
                }
                _ => return Some(path),
            }
        }
    }
    // Deliberately do not fall back to an arbitrary system Chrome in CI: a
    // browser-version difference would turn a backend regression into an
    // unrelated red job. Set MIX_BROWSER_EXECUTABLE or provide the bundled
    // Chromium to run this suite.
    None
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mixd-browser-integration-{tag}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Serve a tiny two-page site on an ephemeral loopback port.
fn serve_fixture() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let request = String::from_utf8_lossy(&buffer);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            let title = if path.starts_with("/b") {
                "Page B"
            } else {
                "Page A"
            };
            let body = format!(
                "<!doctype html><html><head><title>{title}</title></head>\
                 <body><h1>{title}</h1>\
                 <button id=\"go\" onclick=\"document.title='clicked'\">Go</button>\
                 <input id=\"q\">\
                 <select id=\"pick\"><option value=\"a\">A</option><option value=\"b\">B</option></select>\
                 <div id=\"out\"></div></body></html>"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}")
}

fn backend(dir: &Path, executable: &Path) -> mixd_browser::ChromiumBackend {
    let profile = dir.join("profile");
    let artifacts = dir.join("artifacts");
    std::fs::create_dir_all(&profile).expect("profile dir");
    std::fs::create_dir_all(&artifacts).expect("artifact dir");
    mixd_browser::ChromiumBackend::new(
        Some(executable.to_path_buf()),
        Some(profile),
        artifacts,
        true,
    )
}

async fn invoke(
    backend: &mixd_browser::ChromiumBackend,
    workspace: uuid::Uuid,
    action: BrowserAction,
) -> mixd_browser::BrowserResponse {
    backend
        .invoke(BrowserCall {
            workspace_id: workspace,
            action,
        })
        .await
        .expect("browser call")
}

#[tokio::test]
async fn navigate_click_type_and_tabs_use_the_active_page() {
    let _serial = serial().await;
    let Some(executable) = chromium() else {
        if std::env::var("MIX_REQUIRE_BROWSER_TEST").is_ok() {
            panic!("no Chromium available for the browser integration test");
        }
        eprintln!("skipping browser integration test: no Chromium");
        return;
    };

    let dir = temp_dir("main");
    let backend = backend(&dir, &executable);
    backend.set_allow_local(true);
    let workspace = uuid::Uuid::new_v4();
    let base = serve_fixture();

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Navigate {
            url: format!("{base}/a"),
        },
    )
    .await;
    assert!(response.ok, "navigate failed: {:?}", response.error);

    // A second navigation must reuse the active tab and subsequent actions
    // must run against it. This is the regression that "first page in a
    // HashMap" used to cause.
    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Navigate {
            url: format!("{base}/b"),
        },
    )
    .await;
    assert!(response.ok, "second navigate failed: {:?}", response.error);

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Evaluate {
            script: "document.title".into(),
        },
    )
    .await;
    assert!(response.ok);
    assert_eq!(response.payload, serde_json::json!("Page B"));

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Click {
            selector: "#go".into(),
            reference: None,
        },
    )
    .await;
    assert!(response.ok, "click failed: {:?}", response.error);
    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Evaluate {
            script: "document.title".into(),
        },
    )
    .await;
    assert_eq!(response.payload, serde_json::json!("clicked"));

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Type {
            selector: "#q".into(),
            text: "hello".into(),
            reference: None,
            clear: true,
        },
    )
    .await;
    assert!(response.ok, "type failed: {:?}", response.error);
    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Evaluate {
            script: "document.querySelector('#q').value".into(),
        },
    )
    .await;
    assert_eq!(response.payload, serde_json::json!("hello"));

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Snapshot { max_nodes: 50 },
    )
    .await;
    assert!(response.ok, "snapshot failed: {:?}", response.error);
    assert!(
        response.payload["count"].as_u64().unwrap_or(0) > 0,
        "snapshot returned no nodes: {:?}",
        response.payload
    );

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::WaitFor {
            condition: WaitCondition::Text {
                text: "Page B".into(),
            },
            timeout_ms: 2_000,
        },
    )
    .await;
    // The title was changed by the click, but the h1 still says Page B.
    assert!(response.ok, "wait_for(text) failed: {:?}", response.error);

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Screenshot { full_page: false },
    )
    .await;
    assert!(response.ok, "screenshot failed: {:?}", response.error);
    let artifact_id = response.artifact_id.expect("artifact id");
    let png = std::fs::read(dir.join("artifacts").join(format!("{artifact_id}.png")))
        .expect("screenshot file");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "screenshot is not a PNG"
    );

    let (frame, width, height) = backend.capture_frame(false).await.expect("live frame");
    assert!(!frame.is_empty());
    assert!(width > 0 && height > 0, "live frame has no CSS viewport");

    let response = invoke(&backend, workspace, BrowserAction::Tabs).await;
    assert!(response.ok, "tabs failed: {:?}", response.error);
    let tabs = response.payload["tabs"].as_array().expect("tabs");
    // Chromium starts with an initial about:blank tab. The important property
    // is that two navigations did not add one tab per call.
    assert!(
        tabs.len() <= 2,
        "navigate leaked a tab per call: {}",
        tabs.len()
    );
    assert_eq!(
        tabs.iter().filter(|tab| tab["active"] == true).count(),
        1,
        "exactly one tab must be active"
    );

    // `new_tab` is a second navigation entry point and must not accept the
    // non-web schemes a page cannot reach on its own. A `data:` page would run
    // script outside the `evaluate` gate; a `file:` page could be read back
    // with `dom` or `screenshot`.
    for url in [
        "file:///etc/passwd",
        "data:text/html,<h1>not a web page</h1>",
        "javascript:alert(1)",
    ] {
        let response = invoke(
            &backend,
            workspace,
            BrowserAction::NewTab {
                url: Some(url.to_string()),
            },
        )
        .await;
        assert!(!response.ok, "{url} must be refused");
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.contains("blocked")),
            "{url}: unexpected error {:?}",
            response.error
        );
    }

    let response = invoke(&backend, workspace, BrowserAction::Close).await;
    assert!(response.ok);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn local_network_is_blocked_until_opted_in() {
    let _serial = serial().await;
    let Some(executable) = chromium() else {
        eprintln!("skipping browser local-network test: no Chromium");
        return;
    };
    let dir = temp_dir("local");
    let backend = backend(&dir, &executable);
    let workspace = uuid::Uuid::new_v4();
    let base = serve_fixture();

    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Navigate {
            url: format!("{base}/a"),
        },
    )
    .await;
    assert!(!response.ok, "loopback must be blocked by default");
    assert!(
        response
            .error
            .as_deref()
            .is_some_and(|error| error.contains("blocked")),
        "unexpected error: {:?}",
        response.error
    );

    backend.set_allow_local(true);
    let response = invoke(
        &backend,
        workspace,
        BrowserAction::Navigate {
            url: format!("{base}/a"),
        },
    )
    .await;
    assert!(
        response.ok,
        "local opt-in did not allow loopback: {:?}",
        response.error
    );

    let _ = invoke(&backend, workspace, BrowserAction::Close).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn profiles_survive_a_new_operator_instance() {
    let dir = temp_dir("profiles");
    let workspace = uuid::Uuid::new_v4();
    {
        let operator = BrowserOperator::with_resources(&dir, None, true);
        operator
            .ensure_workspace(workspace, "default")
            .await
            .expect("create profile");
        assert!(operator
            .list_profiles()
            .await
            .iter()
            .any(|profile| profile.workspace_id == workspace));
    }
    let operator = BrowserOperator::with_resources(&dir, None, true);
    assert!(
        operator
            .list_profiles()
            .await
            .iter()
            .any(|profile| profile.workspace_id == workspace),
        "a restart must not forget profiles that are still on disk"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
