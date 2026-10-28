//! Native application menu.
//!
//! Builds the platform menu bar and re-emits every activation to the WebView,
//! where the frontend owns the mapping from ids to views and Settings tabs, so
//! the menu and the in-app nav cannot drift apart.

use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

/// Menu event ids, mirrored by the frontend router.
pub const EVT_NEW_CHAT: &str = "menu://new-chat";
pub const EVT_OPEN_SETTINGS: &str = "menu://settings";
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
    // Avoid chords owned by the WebView: ⌘B toggles the sidebar and ⇧⌘D
    // switches surfaces. The menu used to win those, making the shortcut sheet
    // lie and sometimes running an action twice.
    let backup = item(app, EVT_BACKUP, "Create Backup…", Some("CmdOrCtrl+Shift+S"))?;
    let diagnostics = item(app, EVT_DIAGNOSTICS, "Diagnostics", Some("CmdOrCtrl+Alt+D"))?;
    let quit = PredefinedMenuItem::quit(app, Some("Quit MIX-agent"))?;
    let sep = || PredefinedMenuItem::separator(app);

    #[cfg(target_os = "macos")]
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[&new_chat, &sep()?, &backup, &sep()?, &quit],
    )?;
    #[cfg(not(target_os = "macos"))]
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[&new_chat, &settings, &sep()?, &backup, &sep()?, &quit],
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

    let help = Submenu::with_items(app, "Help", true, &[&diagnostics])?;

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
    forward_activations(app);
    Ok(())
}

/// Re-emit every application-menu activation to the WebView under its item id.
/// Tauri delivers menu events to Rust listeners only, so before this the menu
/// was inert: "Settings…" / ⌘, "New Chat" / ⌘N, "Create Backup…" / ⌘B and
/// "Diagnostics" / ⌘⇧D all rendered and claimed their accelerators, yet a click
/// or keystroke never reached the page — the frontend listens for `menu://…`
/// events that nothing emitted.
fn forward_activations<R: Runtime>(app: &AppHandle<R>) {
    app.on_menu_event(|handle, event| {
        let id: &str = event.id().as_ref();
        if !id.starts_with("menu://") {
            // The tray menu's ids belong to `tray::install`.
            return;
        }
        if let Err(err) = handle.emit(id, ()) {
            tracing::warn!(%err, id, "could not forward a menu activation to the WebView");
        }
    });
}
