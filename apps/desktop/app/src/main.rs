//! Tauri 2 binary entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mix_agent_desktop::run();
}
