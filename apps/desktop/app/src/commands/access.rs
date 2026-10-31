//! Access checks shared by commands that accept a filesystem path.
//!
//! A path the user picked through a native dialog is already granted. Every
//! other path — typed into a field, restored from an older client, or invented
//! by a compromised renderer — gets an explicit, native confirmation that
//! names the real path. Saying no is an error, never a silent success.

use std::path::Path;

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::AppState;

/// Require access to `path`, asking the user once if it was not picked.
///
/// `action` is a short Japanese verb phrase ("読み込む", "書き込む") used in
/// the prompt so the dialog says what the app is about to do.
pub fn ensure_access(
    app: &AppHandle,
    state: &AppState,
    path: &Path,
    action: &str,
) -> Result<(), String> {
    if state.grants.is_granted(path) {
        return Ok(());
    }
    let display = path.display();
    let allowed = app
        .dialog()
        .message(format!(
            "MIX-agent が次のパスを{action}することを許可しますか？\nAllow MIX-agent to access this path?\n\n{display}"
        ))
        .title("MIX-agent")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancel)
        .blocking_show();
    if allowed {
        state.grants.grant(path);
        Ok(())
    } else {
        Err(format!("アクセスが許可されませんでした: {display}"))
    }
}

/// Same as [`ensure_access`] but for a folder root that other paths are
/// resolved against. An unapproved root must fail even if the command would
/// otherwise only list one level.
pub fn ensure_root(app: &AppHandle, state: &AppState, root: &Path) -> Result<(), String> {
    ensure_access(app, state, root, "参照")
}
