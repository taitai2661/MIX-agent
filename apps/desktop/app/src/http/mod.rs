//! Optional local HTTP bridge.
//!
//! The Tauri command surface is the canonical API. This module is reserved for
//! future scenarios where the same Rust core needs to be embedded behind an
//! HTTP server (for example a Tauri WebView-less headless mode).

use std::net::SocketAddr;

pub async fn serve(_addr: SocketAddr) -> anyhow::Result<()> {
    Ok(())
}
