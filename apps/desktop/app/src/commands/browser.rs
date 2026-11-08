//! Tauri commands for the native child-WebView browser.
//!
//! The agent calls [`mixd_browser::BrowserHost`] in-process; these commands are
//! the UI surface (tabs, omnibox, native surface bounds and artifact preview).

use crate::AppState;
use mixd_browser::{BrowserAction, BrowserCall, BrowserHost as _, WaitCondition};
use serde::{Deserialize, Serialize};
use tauri::State;

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
    // Direct UI navigation must apply the same allowlist as an agent call.
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

#[derive(Deserialize)]
pub struct BrowserSurfaceInput {
    pub workspace_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}

/// Position the native child WebView over the React placeholder.
#[tauri::command]
pub async fn browser_surface_bounds(
    state: State<'_, AppState>,
    input: BrowserSurfaceInput,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&input.workspace_id).map_err(crate::error::cmd_err)?;
    state
        .browser
        .set_surface(
            id,
            crate::webview_browser::SurfaceBounds {
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
                visible: input.visible,
            },
        )
        .await;
    Ok(())
}

#[tauri::command]
pub async fn browser_clear_data(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    state
        .browser
        .clear_data(id)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn browser_stop(workspace_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(crate::error::cmd_err)?;
    state
        .browser
        .stop_loading(id)
        .await
        .map_err(crate::error::cmd_err)
}

/// Read a screenshot captured by the agent as a data URL.
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
    Ok(format!(
        "data:{};base64,{}",
        artifact.mime_type,
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

async fn gate_navigation(
    egress: &mixd_tools::EgressPolicy,
    url: &str,
    allow_local: bool,
) -> Result<(), String> {
    egress
        .resolve_async_with(url, allow_local)
        .await
        .map_err(crate::error::cmd_err)?;
    mixd_browser::reject_private_navigate_url_with(url, allow_local).map_err(crate::error::cmd_err)
}
