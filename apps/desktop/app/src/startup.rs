/// Bundle identifier from `app/tauri.conf.json`.
///
/// Tauri's `PathResolver::app_data_dir` resolves to
/// `dirs::data_dir().join(identifier)`; tracing needs the same directory before
/// the `AppHandle` exists, so the value is repeated here (the test at the
/// bottom of this file keeps it in sync with the config file).
///
/// Single-sourced: this aliases the Keychain service name in `mixd_storage`
/// (the dependency runs app → storage, so storage cannot alias us). The
/// three-way test below asserts the config file, this constant, and the
/// Keychain service name all agree.
pub const APP_IDENTIFIER: &str = mixd_storage::SERVICE;

/// Console + rolling file logging.
///
/// The file layer writes into the app-data directory with daily rotation, so a
/// crash that takes the WebView down still leaves a trail behind; the console
/// layer keeps the exact behaviour `RUST_LOG` users had before. The directory
/// is resolved through `dirs` rather than `PathResolver` because the
/// subscriber has to be installed before `tauri::Builder` runs. If the
/// directory cannot be created we fall back to console-only instead of losing
/// logging altogether.
pub(crate) fn init_tracing() {
    use tracing_subscriber::layer::{Layer, SubscriberExt};
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let console = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_filter(filter.clone());

    let Some(dir) = dirs::data_dir().map(|base| base.join(APP_IDENTIFIER)) else {
        tracing_subscriber::registry().with(console).init();
        return;
    };
    if let Err(err) = std::fs::create_dir_all(&dir) {
        // No subscriber is installed yet, so `tracing` would swallow this.
        eprintln!(
            "tracing: cannot create the log directory {}: {err}; console logging only",
            dir.display()
        );
        tracing_subscriber::registry().with(console).init();
        return;
    }
    // Provider error bodies and tool output can contain user data. Keep the
    // directory private to the current user; the daily files inherit the
    // protection because another user cannot traverse this directory.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }

    prune_old_logs(&dir);
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(&dir, "mix-agent.log"));
    // A `WorkerGuard` must outlive every log record; storing it in a `static`
    // is what keeps the background writer from shutting down mid-process.
    static LOG_GUARD: std::sync::OnceLock<tracing_appender::non_blocking::WorkerGuard> =
        std::sync::OnceLock::new();
    let _ = LOG_GUARD.set(guard);

    let file = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(writer)
        .with_filter(filter);
    tracing_subscriber::registry()
        .with(console)
        .with(file)
        .init();
}

/// Keep the daily log files bounded. `rolling::daily` never deletes anything
/// on its own, so a long-running install accumulated a file per day forever.
pub(crate) fn prune_old_logs(dir: &std::path::Path) {
    const KEEP: usize = 14;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("mix-agent.log") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    logs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in logs.into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

/// Log a panic before the process unwinds.
///
/// A beta build can abort on a background worker whose panic would otherwise
/// only reach stderr — invisible when the app was launched from Finder. The
/// default hook still runs so debug builds keep their usual output.
pub(crate) fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        // `eprintln!` is unbuffered enough to survive `abort`; the tracing
        // layer is non-blocking and may lose its last record on a hard exit.
        eprintln!("MIX-agent Desktop panicked at {location}: {payload}");
        tracing::error!(%location, %payload, "uncaught panic");
        default_hook(info);
    }));
}
