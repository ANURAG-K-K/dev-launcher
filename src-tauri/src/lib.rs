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
use tauri::{Manager, RunEvent};

use process_manager::ProcessManager;

/// Backend state shared across commands (Tauri managed state).
pub struct AppState {
    pub pool: SqlitePool,
    pub process_manager: ProcessManager,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
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

            let process_manager = ProcessManager::new(app.handle().clone());
            app.manage(AppState {
                pool,
                process_manager,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_project,
            commands::scan_repositories,
            commands::list_recent_projects,
            commands::update_repository_config,
            commands::set_repository_enabled,
            commands::list_dependencies,
            commands::set_repository_dependencies,
            commands::execute_project,
            commands::stop_all,
            commands::list_profiles,
            commands::create_profile,
            commands::update_profile,
            commands::delete_profile,
            commands::apply_profile,
            commands::start_repo,
            commands::stop_repo,
            commands::restart_repo,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // App-quit cleanup: kill every tracked process tree so no dev-server child is
        // orphaned when the launcher exits (F9 / ADR-0003).
        if let RunEvent::ExitRequested { .. } = event {
            if let Some(state) = app_handle.try_state::<AppState>() {
                state.process_manager.kill_all();
            }
        }
    });
}
