//! Native file pickers that also record the user's choice as a trusted path.
//!
//! The frontend must not use `@tauri-apps/plugin-dialog` for paths it later
//! hands back to a file command: the plugin returns the path to JavaScript,
//! and the Rust side cannot tell that value apart from one the WebView made up.
//! Opening the dialog here lets the process that owns the trust boundary see
//! the result directly and grant exactly that path.
//!
//! The callback (`pick_*`) API is used instead of `blocking_pick_*` so a modal
//! does not pin a Tauri async-runtime thread while the user decides.

use serde::Deserialize;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder, FilePath};
use tokio::sync::oneshot;

use crate::AppState;

#[derive(Deserialize)]
pub struct PickerFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

fn apply_filters<R: Runtime>(
    mut builder: FileDialogBuilder<R>,
    title: Option<String>,
    filters: Option<Vec<PickerFilter>>,
) -> FileDialogBuilder<R> {
    if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
        builder = builder.set_title(title);
    }
    for filter in filters.unwrap_or_default() {
        if filter.extensions.is_empty() {
            continue;
        }
        let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
        builder = builder.add_filter(filter.name, &extensions);
    }
    builder
}

async fn pick_one<R: Runtime>(builder: FileDialogBuilder<R>) -> Option<FilePath> {
    let (tx, rx) = oneshot::channel();
    builder.pick_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten()
}

async fn pick_many<R: Runtime>(builder: FileDialogBuilder<R>) -> Vec<FilePath> {
    let (tx, rx) = oneshot::channel();
    builder.pick_files(move |paths| {
        let _ = tx.send(paths);
    });
    rx.await.ok().flatten().unwrap_or_default()
}

async fn pick_folder_path<R: Runtime>(builder: FileDialogBuilder<R>) -> Option<FilePath> {
    let (tx, rx) = oneshot::channel();
    builder.pick_folder(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten()
}

async fn pick_save_path<R: Runtime>(builder: FileDialogBuilder<R>) -> Option<FilePath> {
    let (tx, rx) = oneshot::channel();
    builder.save_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten()
}

/// Pick one or more files. Returns an empty list when the user cancels, which
/// is how the plugin's own picker behaves for a multi-select.
#[tauri::command]
pub async fn pick_open_file(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
    filters: Option<Vec<PickerFilter>>,
    multiple: Option<bool>,
) -> Result<Vec<String>, String> {
    let builder = apply_filters(app.dialog().file(), title, filters);
    let picked = if multiple.unwrap_or(false) {
        pick_many(builder).await
    } else {
        pick_one(builder).await.into_iter().collect()
    };
    let mut out = Vec::new();
    for path in picked {
        let path = path.into_path().map_err(|e| e.to_string())?;
        state.grants.grant(&path);
        out.push(path.to_string_lossy().to_string());
    }
    Ok(out)
}

/// Pick a folder (a project root, a skill directory, the workspace picker).
#[tauri::command]
pub async fn pick_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<Option<String>, String> {
    let builder = apply_filters(app.dialog().file(), title, None);
    let Some(path) = pick_folder_path(builder).await else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    state.grants.grant(&path);
    Ok(Some(path.to_string_lossy().to_string()))
}

/// Pick a save destination. The file may not exist yet, so the grant is keyed
/// by the canonical parent plus the chosen file name.
#[tauri::command]
pub async fn pick_save_file(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
    default_name: Option<String>,
    filters: Option<Vec<PickerFilter>>,
) -> Result<Option<String>, String> {
    let builder = apply_filters(app.dialog().file(), title, filters);
    let builder = match default_name.filter(|n| !n.trim().is_empty()) {
        Some(name) => builder.set_file_name(name),
        None => builder,
    };
    let Some(path) = pick_save_path(builder).await else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    state.grants.grant(&path);
    Ok(Some(path.to_string_lossy().to_string()))
}
