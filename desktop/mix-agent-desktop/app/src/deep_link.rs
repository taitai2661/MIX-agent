//! Custom URL scheme handling.
//!
//! The app registers `mix-agent-desktop://`, which is the OAuth redirect target
//! for remote MCP servers. Without this listener the consent screen would hand
//! the code back to the OS and the flow would never complete.

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_deep_link::DeepLinkExt;

/// Handle one `mix-agent-desktop://` URL, if it is an OAuth redirect.
async fn dispatch<R: Runtime>(app: &AppHandle<R>, url: &str) {
    if !url.starts_with("mix-agent-desktop://") {
        return;
    }
    let state = app.state::<crate::AppState>();
    if let Err(err) = crate::commands::mcp::complete_oauth_flow(&state, url).await {
        tracing::warn!(?url, %err, "deep link did not complete an OAuth flow");
        return;
    }
    tracing::info!(?url, "MCP OAuth callback handled");

    // Surface the result in the UI and republish the server's tools.
    use tauri::Emitter;
    let _ = app.emit("mcp_oauth_complete", url);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::AppState>();
        match mixd_mcp::hydrate_and_connect(&state.mcp, state.storage.as_ref()).await {
            Ok(count) => {
                let _ = app.emit("mcp_tools_changed", count);
            }
            Err(err) => tracing::warn!(?err, "reconnect after OAuth failed"),
        }
    });
}

/// Subscribe to deep links and also drain the URL the app was launched with.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        let handle = handle.clone();
        for url in event.urls() {
            let handle = handle.clone();
            let url = url.to_string();
            tauri::async_runtime::spawn(async move {
                dispatch(&handle, &url).await;
            });
        }
    });

    // Windows / Linux deliver the link as a CLI argument, which only shows up
    // via get_current() at startup.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        match handle.deep_link().get_current() {
            Ok(Some(urls)) => {
                for url in urls {
                    dispatch(&handle, url.as_str()).await;
                }
            }
            Ok(None) => {}
            Err(err) => tracing::warn!(?err, "could not read the launch deep link"),
        }
    });
}
