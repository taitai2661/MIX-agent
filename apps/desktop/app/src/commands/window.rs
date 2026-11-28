use tauri::AppHandle;

/// Open a second application window from the UI (command palette / sidebar).
#[tauri::command]
pub async fn new_window(app: AppHandle) -> Result<(), String> {
    crate::window::open(&app)
        .map(|_| ())
        .map_err(|err| format!("could not open a new window: {err}"))
}
