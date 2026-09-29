//! Workspace-scoped read helpers for the UI.
//!
//! The WebView is denied direct `fs` plugin access; these commands are the
//! surface it uses instead. Path resolution mirrors the `files` Tool
//! (`mixd_tools::files`): the root is the active project's folder when one is
//! given, otherwise the app's current directory, and every path is checked
//! against traversal.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::diff;
use crate::AppState;

#[derive(Deserialize)]
pub struct DiffInput {
    pub path: String,
    pub content: String,
    /// Project root to resolve `path` against; `None` → process CWD.
    #[serde(default)]
    pub root_path: Option<String>,
}

#[derive(Serialize)]
pub struct FileDiffView {
    pub path: String,
    pub unified_diff: String,
    pub added: u32,
    pub removed: u32,
    pub is_new: bool,
    /// True when the preview is degraded (binary old content or read error).
    pub truncated: bool,
}

/// Preview what a `files` write would change, without touching the file.
#[tauri::command]
pub async fn preview_file_diff(
    _state: State<'_, AppState>,
    input: DiffInput,
) -> Result<FileDiffView, String> {
    let path = input.path.trim();
    if path.is_empty() {
        return Err("path required".to_string());
    }
    let root = mixd_tools::files::FilesTool::resolve_context_root(input.root_path.as_deref())
        .map_err(crate::error::cmd_err)?;
    let target =
        mixd_tools::files::FilesTool::safe_join(&root, path).map_err(crate::error::cmd_err)?;

    let exists = tokio::fs::metadata(&target).await.is_ok();
    let read = if exists {
        tokio::fs::read_to_string(&target).await
    } else {
        Ok(String::new())
    };
    let (old, truncated) = match read {
        Ok(content) => (content, false),
        // Non-UTF-8 (binary) old content: degrade to a whole-file replacement.
        Err(_) => (String::new(), true),
    };

    let result = diff::unified_diff(path, &old, &input.content, 3);
    Ok(FileDiffView {
        path: path.to_string(),
        unified_diff: result.unified,
        added: result.added,
        removed: result.removed,
        is_new: result.is_new && !exists,
        truncated,
    })
}

/// Hard cap for user-picked attachments (the model context is the real limit).
const MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Serialize)]
pub struct AttachmentPayload {
    pub kind: String,
    pub mime_type: String,
    pub data_base64: String,
    pub filename: String,
}

/// Read a file the user picked with the OS dialog so it can be attached to a
/// message. The path never comes from the WebView's own logic, only from the
/// dialog result, and the size is capped.
#[tauri::command]
pub async fn read_attachment(path: String) -> Result<AttachmentPayload, String> {
    let path = std::path::PathBuf::from(path.trim());
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "attachment".to_string());
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|e| format!("cannot read {filename}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{filename} is not a file"));
    }
    if meta.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "{filename} is {} MB; the limit is 8 MB",
            meta.len() / (1024 * 1024)
        ));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("cannot read {filename}: {e}"))?;
    let mime = mime_from_name(&filename);
    let kind = if mime.starts_with("image/") {
        "image"
    } else {
        "file"
    };
    Ok(AttachmentPayload {
        kind: kind.to_string(),
        mime_type: mime.to_string(),
        data_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
        filename,
    })
}

fn mime_from_name(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "txt" | "md" | "log" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Open an http(s) URL in the user's default browser. Only the scheme is
/// validated; the launcher is the platform's own URL handler.
#[tauri::command]
pub async fn open_external(url: String) -> Result<(), String> {
    let parsed = url::Url::parse(url.trim()).map_err(crate::error::cmd_err)?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err("only http(s) URLs can be opened".to_string());
    }
    let url = parsed.to_string();
    #[cfg(target_os = "macos")]
    let mut command = tokio::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = tokio::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    command
        .arg(&url)
        .spawn()
        .map_err(|e| format!("failed to open URL: {e}"))?;
    Ok(())
}

/// Write UTF-8 text to a path the user picked with the OS save dialog (used
/// by the Markdown export). Parent directories must already exist.
#[tauri::command]
pub async fn save_text_file(path: String, content: String) -> Result<u64, String> {
    let path = std::path::PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err("path required".to_string());
    }
    let bytes = content.len() as u64;
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(bytes)
}

/// Write base64-decoded bytes to a path from the OS save dialog (used by the
/// skill ZIP export).
#[tauri::command]
pub async fn save_base64_file(path: String, data_base64: String) -> Result<u64, String> {
    let path = std::path::PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err("path required".to_string());
    }
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data_base64)
        .map_err(|e| format!("invalid base64: {e}"))?;
    let len = bytes.len() as u64;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(len)
}
