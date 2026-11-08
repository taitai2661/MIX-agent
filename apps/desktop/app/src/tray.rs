//! System tray icon.
//!
//! Keeps the app reachable when the window is closed, which is what a coding
//! agent wants: long runs continue in the background and the tray is how you
//! get back to them.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

pub const EVT_SHOW: &str = "tray://show";
pub const EVT_QUIT: &str = "tray://quit";

pub fn install<R: Runtime>(app: &AppHandle<R>, language: &str) -> tauri::Result<()> {
    let ja = language == "ja";
    let show = MenuItem::with_id(
        app,
        EVT_SHOW,
        if ja {
            "MIX-agent を開く"
        } else {
            "Open MIX-agent"
        },
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        EVT_QUIT,
        if ja { "終了" } else { "Quit" },
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(app, &[&show, &PredefinedMenuItem::separator(app)?, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("MIX-agent Desktop")
        .menu(&menu)
        // Left click should surface the window, which is the common case.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            EVT_SHOW => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            // Route through the same guard as Cmd+Q: a live Run must be
            // confirmed before the tray tears the process down.
            EVT_QUIT => crate::request_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Surface the pending-approval count where the user already looks.
///
/// The notification only fires once; without a persistent badge an approval
/// requested while the window was hidden was easy to forget. macOS gets the
/// menu-bar number, every platform gets a tooltip.
pub fn update_status<R: Runtime>(app: &AppHandle<R>, pending: usize) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let title = if pending > 0 {
            Some(pending.to_string())
        } else {
            None
        };
        let _ = tray.set_title(title);
    }
    let tooltip = if pending > 0 {
        format!("MIX-agent Desktop — 承認待ち {pending} 件")
    } else {
        "MIX-agent Desktop".to_string()
    };
    let _ = tray.set_tooltip(Some(tooltip));
}
