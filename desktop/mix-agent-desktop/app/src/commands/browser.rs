use crate::AppState;
use mixd_browser::{BrowserAction, BrowserCall, WaitCondition};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct BrowserStatus {
    pub backend: String,
    pub headless: bool,
    pub executable: Option<String>,
    pub profiles: usize,
    pub artifacts: usize,
}

#[tauri::command]
pub async fn browser_status(state: State<'_, AppState>) -> Result<BrowserStatus, String> {
    let (backend, executable) = state.browser.resolve_executable();
    let profiles = state.browser.list_profiles().await.len();
    let artifacts = state.browser.artifacts().await.len();
    Ok(BrowserStatus {
        backend: format!("{:?}", backend).to_lowercase(),
        headless: cfg!(debug_assertions),
        executable: executable.map(|p| p.to_string_lossy().to_string()),
        profiles,
        artifacts,
    })
}

#[derive(Deserialize)]
pub struct BrowserInvokeInput {
    pub workspace_id: Option<String>,
    pub action: String,
    pub url: Option<String>,
    pub selector: Option<String>,
    pub text: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub script: Option<String>,
    pub full_page: Option<bool>,
    pub condition: Option<String>,
    pub timeout_ms: Option<u64>,
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
        },
        "type" => BrowserAction::Type {
            selector: text(&input.selector),
            text: text(&input.text),
        },
        "scroll" => BrowserAction::Scroll {
            x: input.x.unwrap_or(0),
            y: input.y.unwrap_or(0),
        },
        "dom" => BrowserAction::Dom {
            selector: input.selector.clone(),
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
                "function" => WaitCondition::Function {
                    script: text(&input.script),
                },
                _ => WaitCondition::Selector {
                    selector: text(&input.selector),
                },
            };
            BrowserAction::WaitFor {
                condition,
                timeout_ms: input.timeout_ms.unwrap_or(10_000),
            }
        }
        "info" => BrowserAction::Info,
        "close" => BrowserAction::Close,
        other => return Err(format!("unknown browser action: {}", other)),
    };
    let response = state
        .browser
        .invoke(BrowserCall {
            workspace_id,
            action,
        })
        .await
        .map_err(|e| e.to_string())?;
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
    let id = uuid::Uuid::parse_str(&workspace_id).map_err(|e| e.to_string())?;
    state
        .browser
        .reset_profile(id)
        .await
        .map_err(|e| e.to_string())
}
