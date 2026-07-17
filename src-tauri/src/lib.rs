//! Multi-Repo Dev Launcher — Rust backend entry point.
//!
//! Module layout mirrors the architecture doc (docs/architecture.md §3):
//!   - `scanner`         repository discovery + script detection (F2/F3)
//!   - `process_manager` spawn/track/tree-kill dev processes (F7/F9/F10, ADR-0003)
//!   - `persistence`     SQLite via sqlx, Rust-owned (F13, ADR-0002)
//!   - `launcher`        sequential launcher + dependency ordering (F7/F8)
//!   - `commands`        Tauri command/event IPC surface (architecture §5)
//!   - `notifier`        optional desktop notifications (F15)
//!   - `error`           serializable AppError mapped at the IPC boundary (§9)
#![allow(dead_code)] // scaffold: modules are stubs until implemented.

mod commands;
mod error;
mod launcher;
mod notifier;
mod persistence;
mod process_manager;
mod scanner;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
