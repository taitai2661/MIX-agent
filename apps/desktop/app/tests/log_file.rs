//! Proves the file layer `init_tracing` installs actually reaches disk.
//!
//! `init_tracing` itself cannot be called from a test (it installs a global
//! subscriber, once per process), so this exercises the exact same pieces it
//! wires together — `tracing_appender::rolling::daily` behind a non-blocking
//! writer, fed by a `fmt` layer — against a throwaway directory.

use std::time::{Duration, Instant};

use tracing_subscriber::layer::SubscriberExt;

#[test]
fn the_rolling_file_appender_writes_records_to_disk() {
    let dir = std::env::temp_dir().join(format!("mixd-log-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("mkdir");

    let marker = format!("file appender smoke test {}", uuid::Uuid::new_v4());
    {
        let (writer, guard) =
            tracing_appender::non_blocking(tracing_appender::rolling::daily(&dir, "mix-agent.log"));
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("{marker}");
        });
        // Dropping the guard stops the background worker, which flushes
        // whatever is still queued.
        drop(guard);
    }

    // The worker is asynchronous, so poll rather than assume.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut found = String::new();
    while found.is_empty() && Instant::now() < deadline {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(body) = std::fs::read_to_string(entry.path()) {
                    if body.contains(&marker) {
                        found = body;
                        break;
                    }
                }
            }
        }
        if found.is_empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    assert!(
        !found.is_empty(),
        "no file in {dir:?} contains the record — file logging is not hooked up"
    );
    assert!(found.contains("INFO"), "level missing from: {found}");

    let _ = std::fs::remove_dir_all(&dir);
}
