//! Tauri 2 binary entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = mix_agent_desktop::cli::run_if_requested() {
        std::process::exit(code);
    }
    mix_agent_desktop::run();
}
