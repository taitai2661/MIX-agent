//! User-facing terminal pane.
//!
//! This is deliberately not a new shell implementation: it drives the same
//! [`mixd_tools::terminal::TerminalTool`] the agent uses, with a separate run
//! scope. The permission gate is bypassed because the command is typed by the
//! user in the app's own UI — but the *workspace* still has to be one the user
//! opened, which is what [`crate::path_grants::PathGrants`] checks.

use std::path::PathBuf;

use serde::Deserialize;
use tauri::State;

use crate::AppState;

#[derive(Deserialize)]
pub struct ShellRunInput {
    /// Absolute project root. Must be a granted workspace.
    pub root: String,
    pub command: String,
    /// Optional ceiling; the tool clamps it to its own maximum.
    #[serde(default)]
    pub max_wall_clock_ms: Option<u64>,
}

#[derive(Deserialize)]
pub struct ShellStartInput {
    pub root: String,
    pub command: String,
    #[serde(default)]
    pub max_wall_clock_ms: Option<u64>,
}

#[derive(Deserialize)]
pub struct ShellSendInput {
    pub root: String,
    pub input: String,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

fn require_granted(state: &AppState, root: &str) -> Result<(), String> {
    let path = PathBuf::from(root);
    if !state.grants.is_granted(&path) {
        return Err("workspace is not granted; open the folder first".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn run_shell_command(
    state: State<'_, AppState>,
    input: ShellRunInput,
) -> Result<mixd_tools::terminal::TerminalCommand, String> {
    require_granted(&state, &input.root)?;
    // Clone the registry (Arc-backed) instead of holding the read lock for the
    // whole command: a long build must not block an MCP reconnect's write.
    let registry = state.engine.tools.read().await.clone();
    registry
        .terminal_run_user(
            &input.root,
            &input.command,
            input.max_wall_clock_ms.unwrap_or(60_000),
        )
        .await
        .map_err(crate::error::cmd_err)
}

/// Start a command and return immediately; the WebView polls for output.
#[tauri::command]
pub async fn start_shell_command(
    state: State<'_, AppState>,
    input: ShellStartInput,
) -> Result<mixd_tools::terminal::UserShellStart, String> {
    require_granted(&state, &input.root)?;
    let registry = state.engine.tools.read().await.clone();
    registry
        .terminal_start_user(
            &input.root,
            &input.command,
            input.max_wall_clock_ms.unwrap_or(600_000),
        )
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn poll_shell_command(
    state: State<'_, AppState>,
    root: String,
) -> Result<mixd_tools::terminal::UserShellPoll, String> {
    require_granted(&state, &root)?;
    let registry = state.engine.tools.read().await.clone();
    registry
        .terminal_poll_user(&root)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn interrupt_shell_command(
    state: State<'_, AppState>,
    root: String,
) -> Result<(), String> {
    require_granted(&state, &root)?;
    let registry = state.engine.tools.read().await.clone();
    registry
        .terminal_interrupt_user(&root)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn send_shell_input(
    state: State<'_, AppState>,
    input: ShellSendInput,
) -> Result<mixd_tools::ToolResult, String> {
    require_granted(&state, &input.root)?;
    let registry = state.engine.tools.read().await.clone();
    registry
        .terminal_send_user(&input.root, &input.input, input.wait_ms.unwrap_or(500))
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn shell_history(
    state: State<'_, AppState>,
) -> Result<Vec<mixd_tools::terminal::TerminalCommand>, String> {
    let registry = state.engine.tools.read().await.clone();
    Ok(registry.terminal_history().await)
}
