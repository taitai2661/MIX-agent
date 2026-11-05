//! Tauri commands for the launch-at-login setting.

use tauri::command;

#[command]
pub fn get_autostart() -> bool {
    crate::autostart::enabled()
}

#[command]
pub fn set_autostart(enabled: bool) -> Result<bool, String> {
    crate::autostart::set_enabled(enabled)?;
    Ok(crate::autostart::enabled())
}
