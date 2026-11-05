//! Manual smoke test for the macOS Core Spotlight write path.
//!
//! This is not part of the shipped UI: Core Spotlight refuses to index from a
//! bare `target/debug` executable, so the binary has to be placed inside a
//! minimal `.app` bundle with a `CFBundleIdentifier` and ad-hoc signed. The
//! release workflow does not run it. Kept in-tree so the Phase 0 gate can be
//! repeated on a new macOS without writing another throwaway project.

use mix_agent_desktop::spotlight::index::default_sink;
use mix_agent_desktop::spotlight::payload::IndexItem;

fn main() {
    let sink = default_sink();
    if let Err(reason) = sink.available() {
        println!("UNAVAILABLE: {reason}");
        return;
    }

    let item = IndexItem {
        item_id: "conversation:spotlight-smoke".to_string(),
        domain: "conversations",
        source_id: "spotlight-smoke".to_string(),
        title: "MIX-agent Spotlight smoke".to_string(),
        summary: "A temporary item written by the manual smoke test".to_string(),
        keywords: vec!["MIX-agent".to_string(), "smoke".to_string()],
        body: Some("body text for the Core Spotlight smoke test".to_string()),
        created_at: None,
        updated_at: None,
    };

    match sink.index(&[item]) {
        Ok(()) => println!("INDEX_OK"),
        Err(err) => {
            println!("INDEX_ERROR: {err}");
            return;
        }
    }
    match sink.delete(&["conversation:spotlight-smoke".to_string()]) {
        Ok(()) => println!("DELETE_OK"),
        Err(err) => println!("DELETE_ERROR: {err}"),
    }
}
