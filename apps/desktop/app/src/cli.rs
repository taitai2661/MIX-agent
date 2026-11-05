//! Headless command surface.
//!
//! The Tauri bundle already declared a `doctor` subcommand and a `--verbose`
//! flag, but nothing read the parsed CLI: launching the binary with those
//! arguments just opened the window. This module handles the small set of
//! commands that must work before the WebView starts, so the same installer can
//! be used from a terminal or a support script.
//!
//! `run` / `exec` (a full headless agent) are intentionally not here yet: they
//! need the provider → tool registry → engine wiring that `build_app_state`
//! owns, and shipping a half-wired runner would be worse than no runner.

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Handle `--version`, `--help`, and `--doctor`.
///
/// Returns `Some(exit_code)` when the process should exit, `None` when the GUI
/// should start normally. Unknown arguments (including macOS' Finder `-psn_*`)
/// are ignored so double-clicking the app still works.
pub fn run_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("MIX-agent Desktop {}", env!("CARGO_PKG_VERSION"));
        return Some(0);
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Some(0);
    }
    if args.iter().any(|arg| arg == "--doctor") {
        let json = args.iter().any(|arg| arg == "--json");
        return Some(run_doctor(json));
    }
    None
}

fn print_help() {
    println!(
        "MIX-agent Desktop {}\n\n\
         USAGE:\n    MIX-agent Desktop [--doctor [--json]] [--version] [--help]\n\n\
         OPTIONS:\n\
         \x20   --doctor    Check the data directory, database, keychain and log path\n\
         \x20   --json      Machine-readable doctor output\n\
         \x20   --version   Print the application version\n\
         \x20   --help      Show this help\n\n\
         With no arguments the desktop window starts normally.",
        env!("CARGO_PKG_VERSION")
    );
}

fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MIX_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .map(|base| base.join(crate::APP_IDENTIFIER))
        .unwrap_or_else(|| std::env::temp_dir().join("mix-agent-desktop"))
}

fn run_doctor(json: bool) -> i32 {
    let data_dir = data_dir();
    let db_path = data_dir.join("mix-agent.db");
    let log_dir = dirs::data_dir()
        .map(|base| base.join(crate::APP_IDENTIFIER))
        .unwrap_or_else(|| data_dir.clone());

    let writable = check_writable(&data_dir);
    let db_exists = db_path.is_file();
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let (schema_version, quick_check, db_error) = inspect_database(&db_path);
    let secrets = inspect_secrets(&data_dir);

    let ok = writable && db_error.is_none() && (!db_exists || quick_check.as_deref() == Some("ok"));

    if json {
        println!(
            "{}",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "data_dir": data_dir.display().to_string(),
                "data_dir_writable": writable,
                "log_dir": log_dir.display().to_string(),
                "database": {
                    "path": db_path.display().to_string(),
                    "exists": db_exists,
                    "bytes": db_size,
                    "schema_version": schema_version,
                    "quick_check": quick_check,
                    "error": db_error,
                },
                "secrets": secrets,
                "ok": ok,
            })
        );
    } else {
        println!("MIX-agent Desktop {}", env!("CARGO_PKG_VERSION"));
        println!(
            "data dir : {} ({})",
            data_dir.display(),
            if writable { "writable" } else { "NOT writable" }
        );
        println!("log dir  : {}", log_dir.display());
        match &db_error {
            Some(err) => println!("database : {} (error: {err})", db_path.display()),
            None if db_exists => println!(
                "database : {} ({db_size} bytes, schema v{}, quick_check: {})",
                db_path.display(),
                schema_version.unwrap_or(0),
                quick_check.as_deref().unwrap_or("not run")
            ),
            None => println!("database : {} (not created yet)", db_path.display()),
        }
        println!(
            "secrets  : backend={} reachable={} db_key={}",
            secrets.backend,
            secrets.reachable,
            match secrets.db_key_present {
                Some(true) => "present",
                Some(false) => "missing",
                None => "unknown",
            }
        );
        println!(
            "\n{}",
            if ok {
                "OK — the desktop app should be able to start."
            } else {
                "Problems found. See the lines above and the log directory."
            }
        );
    }

    if ok {
        0
    } else {
        1
    }
}

fn check_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".doctor-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

/// Read-only database inspection. Doctor must never migrate or repair a
/// database it is diagnosing.
fn inspect_database(path: &Path) -> (Option<i64>, Option<String>, Option<String>) {
    if !path.is_file() {
        return (None, None, None);
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => return (None, None, Some(format!("could not start runtime: {err}"))),
    };
    runtime.block_on(async {
        let options = SqliteConnectOptions::new().filename(path).read_only(true);
        let pool = match SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
        {
            Ok(pool) => pool,
            Err(err) => return (None, None, Some(err.to_string())),
        };
        let version = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .ok();
        let checks = sqlx::query_scalar::<_, String>("PRAGMA quick_check")
            .fetch_all(&pool)
            .await;
        let (check, error) = match checks {
            Ok(rows) if rows.iter().all(|row| row == "ok") => (Some("ok".to_string()), None),
            Ok(rows) => (None, Some(rows.join("; "))),
            Err(err) => (None, Some(err.to_string())),
        };
        (version, check, error)
    })
}

#[derive(serde::Serialize)]
struct SecretsReport {
    backend: &'static str,
    reachable: bool,
    db_key_present: Option<bool>,
}

fn inspect_secrets(data_dir: &Path) -> SecretsReport {
    let backend = match std::env::var("MIX_SECRET_BACKEND").as_deref() {
        Ok("keychain") => "keychain",
        Ok("file") => "file",
        _ if cfg!(any(target_os = "macos", target_os = "windows")) => "keychain-seed",
        _ => "file",
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return SecretsReport {
                backend,
                reachable: false,
                db_key_present: None,
            }
        }
    };
    runtime.block_on(async {
        let store = mixd_storage::SecretStore::open(data_dir);
        let reachable = store.probe().await.is_ok();
        let db_key_present = store.db_key_status().await;
        SecretsReport {
            backend,
            reachable,
            db_key_present,
        }
    })
}
