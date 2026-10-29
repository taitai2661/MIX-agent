use std::sync::Arc;

use crate::AppState;
use mixd_browser::{
    BrowserAction, BrowserCall, BrowserInstallState, InstallProgress, WaitCondition,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

/// Progress/completion event for the settings pane. The command also returns
/// the initial state so a caller never races the first event.
pub const BROWSER_INSTALL_EVENT: &str = "browser://install";
/// Live CDP screencast frame. Payload: [`BrowserFrameEventPayload`].
pub const BROWSER_FRAME_EVENT: &str = "browser://frame";

#[derive(Clone, Serialize)]
pub struct BrowserFrameEventPayload {
    pub workspace_id: String,
    pub data: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize)]
pub struct BrowserStatus {
    pub backend: String,
    pub source: String,
    pub executable: Option<String>,
    pub installed_version: Option<String>,
    pub pinned_version: String,
    pub update_available: bool,
    pub install: BrowserInstallState,
    pub profiles: usize,
    pub artifacts: usize,
    pub allow_local: bool,
}

#[tauri::command]
pub async fn browser_status(state: State<'_, AppState>) -> Result<BrowserStatus, String> {
    let (backend, executable, source) = state.browser.resolve();
    let profiles = state.browser.list_profiles().await.len();
    let artifacts = state.browser.artifacts().await.len();
    let update = state.chromium_installer.update_info();
    Ok(BrowserStatus {
        backend: format!("{:?}", backend).to_lowercase(),
        source: format!("{:?}", source).to_lowercase(),
        executable: executable.map(|p| p.to_string_lossy().to_string()),
        installed_version: update.installed_version,
        pinned_version: update.pinned_version,
        update_available: update.update_available,
        install: state.chromium_installer.status().await,
        profiles,
        artifacts,
        allow_local: state.browser.allows_local(),
    })
}

/// Start (or resume) the managed Chromium install.
///
/// Returns the initial state and completes in the background. The browser
/// backends are closed first so a running executable cannot block the file
/// swap on Windows, and the Tool Registry is rebuilt afterwards so a first
/// install makes `browser` visible without an app relaunch.
#[tauri::command]
pub async fn browser_install(
    app: AppHandle,
    state: State<'_, AppState>,
    force: Option<bool>,
) -> Result<BrowserInstallState, String> {
    let force = force.unwrap_or(false);
    let installer = state.chromium_installer.clone();
    let (initial, started) = installer.begin(force).await?;
    if !started || !initial.is_installing() {
        return Ok(initial);
    }

    let app_state = state.inner().clone();
    let run_installer = installer.clone();
    let status_installer = installer.clone();
    let event_app = app.clone();
    let progress: InstallProgress = Arc::new(move |state| {
        let _ = event_app.emit(BROWSER_INSTALL_EVENT, state);
    });

    tauri::async_runtime::spawn(async move {
        // Close running browsers before the swap. The timeout keeps a wedged
        // backend from parking the install forever; on Windows a still-locked
        // executable makes the swap fail with `replace_failed`, which is a
        // clearer outcome than an endless "preparing".
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            app_state.browser.reset_backends(),
        )
        .await;
        run_installer.run(force, Some(progress)).await;
        // Rebuild after both success and failure: a failed install must not
        // leave a stale tool list behind either.
        crate::refresh_registry(&app_state).await;
        let _ = app.emit(BROWSER_INSTALL_EVENT, status_installer.status().await);
    });

    Ok(initial)
}

/// Ask a running install to stop at the next safe boundary.
#[tauri::command]
pub async fn browser_install_cancel(
    state: State<'_, AppState>,
) -> Result<BrowserInstallState, String> {
    state.chromium_installer.cancel();
    Ok(state.chromium_installer.status().await)
}

/// Delete the managed Chromium tree. System/legacy fallbacks remain usable.
#[tauri::command]
pub async fn browser_uninstall(state: State<'_, AppState>) -> Result<(), String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        state.browser.reset_backends(),
    )
    .await
    .map_err(|_| {
        "実行中のChromiumを停止できませんでした。アプリを再起動してください。".to_string()
    })?;
    state.chromium_installer.remove().await?;
    crate::refresh_registry(&state).await;
    Ok(())
}

#[derive(Deserialize)]
pub struct BrowserInvokeInput {
    pub workspace_id: Option<String>,
    pub action: String,
    pub url: Option<String>,
    pub selector: Option<String>,
    pub r#ref: Option<String>,
    pub text: Option<String>,
    pub key: Option<String>,
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub clear: Option<bool>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub script: Option<String>,
    pub full_page: Option<bool>,
    pub condition: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_nodes: Option<usize>,
    pub limit: Option<usize>,
    pub tab_id: Option<String>,
}

#[derive(Serialize)]
pub struct BrowserInvokeOutput {
    pub ok: bool,
    pub payload: serde_json::Value,
    pub artifact_id: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn browser_invoke(
    state: State<'_, AppState>,
    input: BrowserInvokeInput,
) -> Result<BrowserInvokeOutput, String> {
    let workspace_id = input
        .workspace_id
        .as_deref()
        .and_then(|w| uuid::Uuid::parse_str(w).ok())
        .unwrap_or_else(uuid::Uuid::new_v4);
    let text = |v: &Option<String>| v.clone().unwrap_or_default();
    let action = match input.action.as_str() {
        "navigate" => BrowserAction::Navigate {
            url: text(&input.url),
        },
        "click" => BrowserAction::Click {
            selector: text(&input.selector),
            reference: input.r#ref.clone(),
        },
        "click_at" => BrowserAction::ClickAt {
            x: input.x.unwrap_or(0),
            y: input.y.unwrap_or(0),
        },
        "type" => BrowserAction::Type {
            selector: text(&input.selector),
            text: text(&input.text),
            reference: input.r#ref.clone(),
            clear: input.clear.unwrap_or(false),
        },
        "type_active" => BrowserAction::TypeActive {
            text: text(&input.text),
        },
        "press" => BrowserAction::Press {
            key: text(&input.key),
        },
        "hover" => BrowserAction::Hover {
            selector: text(&input.selector),
            reference: input.r#ref.clone(),
        },
        "select" => BrowserAction::Select {
            selector: text(&input.selector),
            value: text(&input.value),
            reference: input.r#ref.clone(),
        },
        "check" => BrowserAction::Check {
            selector: text(&input.selector),
            checked: input.checked.unwrap_or(true),
            reference: input.r#ref.clone(),
        },
        "scroll" => BrowserAction::Scroll {
            x: input.x.unwrap_or(0),
            y: input.y.unwrap_or(0),
        },
        "dom" => BrowserAction::Dom {
            selector: input.selector.clone(),
        },
        "extract" => BrowserAction::Extract {
            selector: text(&input.selector),
            limit: input.limit.unwrap_or(20).clamp(1, 100),
        },
        "snapshot" => BrowserAction::Snapshot {
            max_nodes: input.max_nodes.unwrap_or(300).clamp(1, 300),
        },
        "screenshot" => BrowserAction::Screenshot {
            full_page: input.full_page.unwrap_or(false),
        },
        "evaluate" => BrowserAction::Evaluate {
            script: text(&input.script),
        },
        "wait_for" => {
            let condition = match input.condition.as_deref().unwrap_or("selector") {
                "url" => WaitCondition::Url {
                    url: text(&input.url),
                },
                "text" => WaitCondition::Text {
                    text: text(&input.text),
                },
                "load" => WaitCondition::Load,
                "function" => WaitCondition::Function {
                    script: text(&input.script),
                },
                _ => WaitCondition::Selector {
                    selector: text(&input.selector),
                },
            };
            BrowserAction::WaitFor {
                condition,
                timeout_ms: input.timeout_ms.unwrap_or(10_000).min(60_000),
            }
        }
        "info" => BrowserAction::Info,
        "reload" => BrowserAction::Reload,
        "back" => BrowserAction::Back,
        "forward" => BrowserAction::Forward,
        "tabs" => BrowserAction::Tabs,
        "new_tab" => BrowserAction::NewTab {
            url: input.url.clone(),
        },
        "switch_tab" => BrowserAction::SwitchTab {
            tab_id: text(&input.tab_id),
        },
        "close_tab" => BrowserAction::CloseTab {
            tab_id: input.tab_id.clone(),
        },
        "close" => BrowserAction::Close,
        other => return Err(format!("unknown browser action: {}", other)),
    };
    // The manual pane calls the operator directly, so it must apply the same
    // egress allowlist the Tool Registry applies to agent calls. Without this
    // a user-level request could reach a host the agent is forbidden to use.
    // Read the policy under the lock (not a startup copy) so saving the
    // settings form is effective immediately.
    let allow_local = state.browser.allows_local();
    let egress = state.egress.read().await.clone();
    match &action {
        BrowserAction::Navigate { url } => {
            gate_navigation(&egress, url, allow_local).await?;
        }
        BrowserAction::WaitFor {
            condition: WaitCondition::Url { url },
            ..
        } => {
            gate_navigation(&egress, url, allow_local).await?;
        }
        BrowserAction::NewTab { url: Some(url) } => {
            gate_navigation(&egress, url, allow_local).await?;
        }
        _ => {}
    }
    let response = state
        .browser
        .invoke(BrowserCall {
            workspace_id,
            action,
        })
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(BrowserInvokeOutput {
        ok: response.ok,
        payload: response.payload,
        artifact_id: response.artifact_id,
        error: response.error,
    })
}

#[derive(Serialize)]
pub struct BrowserProfileView {
    pub workspace_id: String,
    pub display_name: String,
    pub directory: String,
    pub provisioned: bool,
}

#[tauri::command]
pub async fn browser_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<BrowserProfileView>, String> {
    Ok(state
        .browser
        .list_profiles()
        .await
        .into_iter()
        .map(|p| BrowserProfileView {
            workspace_id: p.workspace_id.to_string(),
            display_name: p.display_name,
            directory: p.directory,
            provisioned: p.provisioned,
        })
        .collect())
}

#[tauri::command]
pub async fn browser_reset_profile(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    state
        .browser
        .reset_profile(id)
        .await
        .map_err(crate::error::cmd_err)
}

/// Read a screenshot (or PDF) captured by the Browser Operator as a data URL.
///
/// The activity log previously showed the artifact id as a dead badge: the
/// file existed on disk but there was no route to it from the UI. Return a data
/// URL so the WebView can render it without exposing the filesystem path.
#[tauri::command]
pub async fn browser_artifact(state: State<'_, AppState>, id: String) -> Result<String, String> {
    use base64::Engine as _;
    let artifacts = state.browser.artifacts().await;
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact.id == id)
        .ok_or_else(|| "artifact not found".to_string())?;
    let bytes = tokio::fs::read(&artifact.path)
        .await
        .map_err(crate::error::cmd_err)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{};base64,{}", artifact.mime_type, encoded))
}

/// Apply the Tool Registry's navigation policy to a manual browser call.
///
/// The operator's backend repeats the literal/DNS check before dialling; this
/// one fails fast and keeps the pane from sending an obviously forbidden URL.
async fn gate_navigation(
    egress: &mixd_tools::EgressPolicy,
    url: &str,
    allow_local: bool,
) -> Result<(), String> {
    if url == "about:blank" {
        return Ok(());
    }
    egress
        .resolve_async_with(url, allow_local)
        .await
        .map_err(crate::error::cmd_err)?;
    mixd_browser::reject_private_navigate_url_with(url, allow_local)
        .map_err(crate::error::cmd_err)?;
    Ok(())
}

/// Capture a fresh, non-persisted JPEG frame for the live-view poller.
///
/// Deliberately separate from `browser_artifact`: polling must not create a
/// new artifact every 1.5 s. `None` means the workspace was busy with an agent
/// action; the UI keeps the previous frame rather than blocking that action.
/// Page metadata comes from `browser_invoke`'s `info` action, so this payload
/// stays image-only.
#[derive(Serialize)]
pub struct BrowserFrameOutput {
    pub data: String,
    pub width: u32,
    pub height: u32,
}

#[tauri::command]
pub async fn browser_frame(
    state: State<'_, AppState>,
    workspace_id: String,
    full_page: Option<bool>,
) -> Result<Option<BrowserFrameOutput>, String> {
    use base64::Engine as _;
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    let Some(frame) = state
        .browser
        .capture_frame(id, full_page.unwrap_or(false))
        .await
        .map_err(crate::error::cmd_err)?
    else {
        return Ok(None);
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(frame.jpeg);
    Ok(Some(BrowserFrameOutput {
        data: format!("data:image/jpeg;base64,{}", encoded),
        width: frame.width,
        height: frame.height,
    }))
}

/// Keep an open panel's workspace out of the idle LRU without launching it.
#[tauri::command]
pub async fn browser_keepalive(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    state.browser.touch_workspace(id).await;
    Ok(())
}

/// Stop loading the active tab from the side-panel toolbar.
#[tauri::command]
pub async fn browser_stop(state: State<'_, AppState>, workspace_id: String) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    state
        .browser
        .stop_loading(id)
        .await
        .map_err(crate::error::cmd_err)
}

/// Start pushing CDP screencast frames (`browser://frame`) for a workspace.
#[tauri::command]
pub async fn browser_screencast_start(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    state
        .browser
        .start_screencast(id)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn browser_screencast_stop(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    state.browser.stop_screencast(id).await;
    Ok(())
}
