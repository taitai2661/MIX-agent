//! Native application menu.
//!
//! Builds the platform menu bar. Event ids are routed by the frontend, which
//! owns the mapping to views and Settings tabs, so the menu and the in-app nav
//! cannot drift apart.

use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

/// Menu event ids, mirrored by the frontend router.
pub const EVT_NEW_CHAT: &str = "menu://new-chat";
pub const EVT_OPEN_SETTINGS: &str = "menu://settings";
pub const EVT_CHECK_UPDATES: &str = "menu://check-updates";
pub const EVT_BACKUP: &str = "menu://backup";
pub const EVT_DIAGNOSTICS: &str = "menu://diagnostics";

fn item<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    text: &str,
    accel: Option<&str>,
) -> tauri::Result<MenuItem<R>> {
    let builder = MenuItem::with_id(app, id, text, true, None::<&str>)?;
    match accel {
        Some(a) => {
            // A rejected accelerator should not stop the menu from appearing.
            if let Err(err) = builder.set_accelerator(Some(a)) {
                tracing::warn!(accelerator = a, %err, "could not set accelerator");
            }
            Ok(builder)
        }
        None => Ok(builder),
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let new_chat = item(app, EVT_NEW_CHAT, "New Chat", Some("CmdOrCtrl+N"))?;
    let settings = item(app, EVT_OPEN_SETTINGS, "Settings…", Some("CmdOrCtrl+,"))?;
    let backup = item(app, EVT_BACKUP, "Create Backup…", Some("CmdOrCtrl+B"))?;
    let diagnostics = item(
        app,
        EVT_DIAGNOSTICS,
        "Diagnostics",
        Some("CmdOrCtrl+Shift+D"),
    )?;
    let check_updates = item(app, EVT_CHECK_UPDATES, "Check for Updates…", None)?;
    let quit = PredefinedMenuItem::quit(app, Some("Quit MIX-agent"))?;
    let sep = || PredefinedMenuItem::separator(app);

    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[&new_chat, &sep()?, &backup, &sep()?, &quit],
    )?;

    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &PredefinedMenuItem::fullscreen(app, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &sep()?,
            &PredefinedMenuItem::show_all(app, None)?,
        ],
    )?;

    let help = Submenu::with_items(app, "Help", true, &[&diagnostics, &sep()?, &check_updates])?;

    // Settings lives in the app menu on macOS and in File elsewhere, so it is
    // reachable on Windows and Linux too.
    #[cfg(target_os = "macos")]
    let app_menu = Submenu::with_items(
        app,
        "MIX-agent",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("MIX-agent"), None)?,
            &sep()?,
            &settings,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &quit,
        ],
    )?;

    // Build the top-level list per platform: `cfg` on an array element is not
    // stable, so the macOS app menu is appended conditionally instead.
    let mut top: Vec<&dyn IsMenuItem<R>> = Vec::new();
    #[cfg(target_os = "macos")]
    top.push(&app_menu);
    top.extend([&file as &dyn IsMenuItem<R>, &edit, &view, &help]);
    let menu = Menu::with_items(app, &top)?;

    app.set_menu(menu)?;
    Ok(())
}
