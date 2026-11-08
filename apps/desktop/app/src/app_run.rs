use super::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    install_panic_hook();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_cli::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Rust-registered quick-input hotkey. The WebView never receives the
        // plugin's IPC surface; it only listens for the emitted event.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // `store`, `process`, `shell` and `http` used to be registered even
        // though no command or UI module called them. Their capabilities were
        // already denied, but not registering them removes the IPC surface
        // entirely rather than relying on the ACL as the only gate.
        // Signed, verified updates. The WebView reaches this only through
        // `commands::update`, so no `updater:*` capability is granted.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            use tauri::Manager;
            let app_handle = app.handle().clone();
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("mix-agent"));
            // Use Tauri's global async runtime rather than a throwaway one: MCP
            // stdio sessions spawn long-lived reader tasks, and those tasks
            // would be torn down when a local runtime is dropped at the end of
            // setup, silently killing every connected server.
            // Startup failures are logged and returned rather than panicked:
            // with `panic = "abort"` an `.expect` here killed the process with
            // no diagnostic a user could act on.
            //
            // Writability pre-check: `Storage::open` also creates the dir, but
            // surfacing a permission failure here names the fix (ownership /
            // ACLs / read-only volume) before SQLite reports a bare I/O error.
            // `block_on` has no timeout on Tauri's runtime, so a wedged open
            // (stale lock, unresponsive volume) would hang the splash screen;
            // the error text below therefore names the remedies (permissions,
            // stale lock/WAL cleanup, restart) instead of just the path.
            if let Err(err) = std::fs::create_dir_all(&data_dir) {
                tracing::error!(%err, dir = %data_dir.display(), "data directory is not writable");
                return Err(Box::<dyn std::error::Error>::from(format!(
                    "could not prepare the data directory at {}: {err}. \
                     Check that the directory is writable (folder permissions / ownership, \
                     read-only volume, full disk). If the app previously crashed, a stale \
                     lock or -wal/-shm sidecar may also wedge startup — removing them and \
                     restarting usually clears it.",
                    data_dir.display()
                )));
            }
            let storage =
                match tauri::async_runtime::block_on(mixd_storage::Storage::open(&data_dir)) {
                    Ok(storage) => std::sync::Arc::new(storage),
                    Err(err) => {
                        tracing::error!(%err, dir = %data_dir.display(), "could not open storage");
                        return Err(Box::<dyn std::error::Error>::from(format!(
                            "could not open storage at {}: {err}. \
                             Check that the directory is writable and not locked by another \
                             instance (a stale mix-agent.db-wal/-shm from a crash can wedge \
                             startup — quit all instances, remove the sidecars, and restart). \
                             If the database itself is corrupt, restore from a backup.",
                            data_dir.display()
                        )));
                    }
                };
            let secrets = std::sync::Arc::new(mixd_storage::SecretStore::open(&data_dir));
            let state = match tauri::async_runtime::block_on(build_app_state(
                app_handle.clone(),
                storage,
                secrets,
                &data_dir,
            )) {
                Ok(state) => state,
                Err(err) => {
                    tracing::error!(%err, "could not build the application state");
                    return Err(Box::<dyn std::error::Error>::from(format!(
                        "could not build the application state: {err}. \
                         This usually means a provider row or MCP config failed to load — \
                         check the log file in the data directory, fix or remove the \
                         offending row, and restart the app."
                    )));
                }
            };
            app.manage(state);
            // Update downloads are staged under the app-data directory and
            // applied when the user quits. The state is separate from
            // `AppState` because the quit hook and the update commands are its
            // only consumers.
            app.manage(commands::update::UpdateState::new(
                data_dir.join("updates"),
                app.package_info().version.to_string(),
            ));
            // Forward Run Engine broadcast events into the WebView.
            let managed = app.state::<AppState>();
            // The everyday database snapshot runs after the window exists; a
            // pending migration already took its blocking copy in
            // `Storage::open`.
            let snapshot_storage = managed.storage.clone();
            tauri::async_runtime::spawn(async move {
                snapshot_storage.snapshot_if_due().await;
                snapshot_storage.maintenance().await;
            });
            run_events::spawn(
                app_handle.clone(),
                managed.engine.clone(),
                managed.spotlight.clone(),
            );
            spotlight::install(&app_handle, managed.spotlight.clone());
            // Language servers and other subprocess-holding tools release
            // themselves after ten idle minutes. The sweep is deliberately
            // low-frequency: it is housekeeping, not a request.
            {
                let engine = managed.engine.clone();
                tauri::async_runtime::spawn(async move {
                    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
                    ticker.tick().await;
                    loop {
                        ticker.tick().await;
                        engine.tools.read().await.sweep_idle().await;
                    }
                });
            }
            // Local runtimes start in the background: a scan or an auto-start
            // must never delay the first paint of the shell.
            let local_runtime = managed.local_runtime.clone();
            tauri::async_runtime::spawn(async move {
                local_runtime.startup().await;
            });
            deep_link::install(&app_handle);
            scheduler::install(&app_handle);
            // Native chrome follows the same language as the WebView. Reading
            // the stored settings is one synchronous SQLite read during setup;
            // it is cheaper than shipping an English menu to a Japanese app.
            let menu_language = tauri::async_runtime::block_on(commands::settings::read_settings(
                app_handle.state::<AppState>().storage.pool(),
            ))
            .map(|settings| settings.ui_language())
            .unwrap_or("en");
            menu::install(&app_handle, menu_language)?;
            tray::install(&app_handle, menu_language)?;
            {
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                let shortcut = "CommandOrControl+Shift+Space";
                if let Err(err) =
                    app_handle
                        .global_shortcut()
                        .on_shortcut(shortcut, |app, _shortcut, event| {
                            if event.state != ShortcutState::Pressed {
                                return;
                            }
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                            let _ = app.emit("shortcut://quick-input", ());
                        })
                {
                    // A desktop environment may already own the chord; the
                    // app still works, so this is a warning, not a startup
                    // failure.
                    tracing::warn!(shortcut, %err, "could not register the quick-input shortcut");
                } else {
                    tracing::info!(shortcut, "registered the quick-input shortcut");
                }
            }
            push::install(&app_handle);
            // Autostart is how a scheduled task survives a reboot. When the
            // OS launches us with `--hidden`, keep the window out of the way:
            // the tray icon and notifications are the intended surface.
            if std::env::args().any(|arg| arg == autostart::HIDDEN_ARG) {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        // Closing the window keeps a live run alive in the tray. Before this,
        // the default "exit when the last window closes" killed streaming runs
        // and made the tray unreachable, contradicting its own documentation.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                let _ = window.app_handle().emit("app://hidden-to-tray", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::health::health,
            commands::autostart::get_autostart,
            commands::autostart::set_autostart,
            commands::providers::list_providers,
            commands::providers::create_provider,
            commands::providers::update_provider,
            commands::providers::delete_provider,
            commands::providers::provider_presets,
            commands::providers::connect_provider,
            commands::chatgpt::chatgpt_status,
            commands::chatgpt::chatgpt_ensure,
            commands::chatgpt::chatgpt_device_start,
            commands::chatgpt::chatgpt_device_poll,
            commands::chatgpt::chatgpt_device_cancel,
            commands::chatgpt::chatgpt_quota,
            commands::chatgpt::chatgpt_logout,
            commands::discovery::scan_local_runtimes,
            commands::discovery::probe_local_endpoint,
            commands::discovery::import_local_runtime,
            commands::local_runtime::local_runtime_snapshot,
            commands::local_runtime::local_runtime_refresh,
            commands::local_runtime::local_runtime_start,
            commands::local_runtime::local_runtime_stop,
            commands::local_runtime::local_runtime_restart,
            commands::local_runtime::local_runtime_logs,
            commands::local_runtime::local_runtime_open_app,
            commands::local_runtime::local_model_pull,
            commands::local_runtime::local_model_delete,
            commands::local_runtime::local_model_load,
            commands::local_runtime::local_model_unload,
            commands::local_runtime::local_job_list,
            commands::local_runtime::local_job_cancel,
            commands::models::list_models,
            commands::models::verify_tools,
            commands::models::set_model_auto,
            commands::auto::resolve_auto,
            commands::auto::set_message_feedback,
            commands::conversations::list_conversations,
            commands::conversations::list_conversations_page,
            commands::conversations::get_conversation,
            commands::conversations::latest_conversation,
            commands::conversations::list_conversations_for_project,
            commands::conversations::search_conversations,
            commands::conversations::conversation_stats,
            commands::conversations::search_messages,
            commands::conversations::create_conversation,
            commands::conversations::send_message,
            commands::conversations::edit_message,
            commands::conversations::list_messages,
            commands::conversations::rename_conversation,
            commands::conversations::fork_conversation,
            commands::conversations::delete_conversation,
            commands::conversations::export_markdown,
            commands::runs::get_run,
            commands::runs::list_runs,
            commands::runs::list_interrupted_runs,
            commands::runs::list_active_runs,
            commands::runs::list_run_events,
            commands::runs::cancel_run,
            commands::runs::quit_app,
            commands::runs::steer_run,
            commands::runs::revert_run,
            commands::runs::run_workspace_changes,
            commands::runs::revert_run_workspace,
            commands::runs::resume_run,
            commands::runs::budget_extension,
            commands::runs::list_checkpoints,
            commands::runs::replay_run_events,
            commands::fs::preview_file_diff,
            commands::fs::read_attachment,
            commands::fs::open_external,
            commands::fs::save_text_file,
            commands::fs::save_base64_file,
            commands::picker::pick_open_file,
            commands::picker::pick_folder,
            commands::picker::pick_save_file,
            commands::approvals::decide_approval,
            commands::tools::list_tools,
            commands::tools::set_permission_rule,
            commands::tools::list_permission_rules,
            commands::tools::list_pending_approvals,
            commands::mcp::list_connections,
            commands::mcp::create_connection,
            commands::mcp::delete_connection,
            commands::mcp::start_oauth,
            commands::mcp::connect,
            commands::mcp::handle_oauth_callback,
            commands::browser::browser_invoke,
            commands::browser::browser_surface_bounds,
            commands::browser::browser_clear_data,
            commands::browser::browser_artifact,
            commands::browser::browser_stop,
            commands::memories::list_memories,
            commands::memories::create_memory,
            commands::memories::update_memory,
            commands::memories::delete_memory,
            commands::memories::list_memory_revisions,
            commands::memories::restore_memory_revision,
            commands::memories::verify_memory,
            commands::memories::search_memories,
            commands::memories::memory_associations,
            commands::memories::resolve_conflict,
            commands::knowledge::list_knowledge,
            commands::knowledge::ingest_knowledge,
            commands::knowledge::import_knowledge_file,
            commands::knowledge::delete_knowledge,
            commands::knowledge::search_knowledge,
            commands::skills::list_skills,
            commands::skills::import_skill,
            commands::skills::export_skill,
            commands::skills::delete_skill,
            commands::skills::list_skill_resources,
            commands::skills::read_skill_resource,
            commands::projects::list_projects,
            commands::projects::create_project,
            commands::projects::update_project,
            commands::projects::delete_project,
            commands::projects::open_project_folder,
            commands::projects::open_project_terminal,
            commands::conversations::set_conversation_project,
            commands::workspace::list_dir,
            commands::workspace::find_workspace_files,
            commands::workspace::read_workspace_file,
            commands::workspace::git_status,
            commands::workspace::git_diff,
            commands::workspace::git_restore,
            commands::workspace::git_stage,
            commands::schedules::list_jobs,
            commands::schedules::create_job,
            commands::schedules::set_job_enabled,
            commands::schedules::preview_cron,
            commands::schedules::delete_job,
            commands::schedules::update_job,
            commands::schedules::run_job_now,
            commands::settings::get_settings,
            commands::settings::get_monthly_spend,
            commands::settings::update_settings,
            commands::settings::sandbox_status,
            commands::spotlight::spotlight_status,
            commands::spotlight::spotlight_rebuild,
            commands::spotlight::spotlight_clear,
            commands::spotlight::spotlight_take_pending,
            commands::agents::list_agents,
            commands::agents::create_agent,
            commands::agents::update_agent,
            commands::agents::delete_agent,
            commands::backups::create_backup,
            commands::backups::restore_backup,
            commands::backups::relaunch_app,
            commands::diagnostics::collect_diagnostics,
            commands::diagnostics::get_recent_logs,
            commands::diagnostics::export_diagnostics,
            commands::diagnostics::diagnostics_bundle,
            commands::update::app_version,
            commands::update::check_for_update,
            commands::update::download_update,
            commands::update::pending_update,
            commands::update::install_update_now,
        ])
        .build(tauri::generate_context!());

    let app = match result {
        Ok(app) => app,
        Err(err) => {
            tracing::error!(%err, "MIX-agent Desktop exited with an error");
            std::process::exit(1);
        }
    };
    app.run(|app_handle, event| {
        // A downloaded update is applied on a normal quit, never by an
        // automatic restart. `prevent_exit` buys the async install time to
        // finish; the second `ExitRequested` (from `exit(0)` below) is let
        // through by the state's `allow_exit` flag. `app.restart()` is exempt:
        // it ignores `prevent_exit`, so an update must not try to run there.
        if let tauri::RunEvent::ExitRequested { code, api, .. } = &event {
            if *code != Some(tauri::RESTART_EXIT_CODE) {
                // A live Run (or a scheduled job) must not disappear because
                // the user hit Cmd+Q or the tray's Quit. Ask the WebView first;
                // `quit_app` sets ALLOW_EXIT once the user has confirmed.
                if !ALLOW_EXIT.load(Ordering::SeqCst) {
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        let active =
                            tauri::async_runtime::block_on(count_active_runs(state.storage.pool()));
                        if active > 0 {
                            api.prevent_exit();
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                            let _ = app_handle
                                .emit(EVT_QUIT_BLOCKED, serde_json::json!({ "count": active }));
                            return;
                        }
                    }
                }
                if let Some(update) = app_handle.try_state::<commands::update::UpdateState>() {
                    if update.should_apply_on_exit() {
                        api.prevent_exit();
                        let handle = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            commands::update::apply_pending_on_exit(handle.clone()).await;
                            handle.exit(0);
                        });
                        return;
                    }
                }
            }
        }
        // Give the runtime manager a chance to stop only the processes it
        // started. `local_stop_on_exit` can keep them alive for a persistent
        // local server; anything launched externally is never touched.
        if matches!(&event, tauri::RunEvent::Exit) {
            if let Some(state) = app_handle.try_state::<AppState>() {
                // One overall deadline: quitting must not block for 5 s per
                // runtime when a child ignores SIGTERM.
                let shutdown = state.local_runtime.shutdown();
                let _ = tauri::async_runtime::block_on(async {
                    tokio::time::timeout(std::time::Duration::from_secs(8), shutdown).await
                });
            }
        }
        // Dock icon / activation on macOS must bring the hidden window back.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } = event
        {
            if !has_visible_windows {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app_handle, event);
    });
}
