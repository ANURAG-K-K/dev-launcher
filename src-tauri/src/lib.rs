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
#![allow(dead_code)] // scaffold: some modules are stubs until later slices.

mod commands;
mod error;
mod launcher;
mod notifier;
mod persistence;
mod process_manager;
mod scanner;

use sqlx::SqlitePool;
use tauri::Manager;

/// Backend state shared across commands (Tauri managed state).
pub struct AppState {
    pub pool: SqlitePool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Resolve the SQLite file under the OS app-data dir (data-model §1) and
            // initialize the pool + run migrations before any command is reachable.
            let db_path = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir")
                .join("app.db");

            let pool = tauri::async_runtime::block_on(async {
                let pool = persistence::init_pool(&db_path)
                    .await
                    .expect("failed to open database");
                persistence::run_migrations(&pool)
                    .await
                    .expect("failed to run migrations");
                pool
            });

            app.manage(AppState { pool });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_project,
            commands::scan_repositories,
            commands::list_recent_projects,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
