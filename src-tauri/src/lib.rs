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
mod git_status_cache;
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
    pub git_status_cache: std::sync::Arc<git_status_cache::GitStatusCache>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
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
                // Reconcile launches left "running" by a previous session (F13).
                let _ = persistence::reconcile_stale_launches(&pool).await;
                pool
            });

            let process_manager = ProcessManager::new(app.handle().clone(), pool.clone());
            app.manage(AppState {
                pool,
                process_manager,
                git_status_cache: std::sync::Arc::new(git_status_cache::GitStatusCache::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_project,
            commands::rename_project,
            commands::scan_repositories,
            commands::refresh_repositories,
            commands::delete_project,
            commands::project_running_counts,
            commands::update_project_path,
            commands::add_repository_manual,
            commands::update_repository_path,
            commands::list_recent_projects,
            commands::get_settings,
            commands::update_settings,
            commands::update_repository_config,
            commands::set_repository_enabled,
            commands::set_repository_favorite,
            commands::set_repository_visible_console,
            commands::remove_repository,
            commands::open_repo_folder,
            commands::list_env_files,
            commands::open_repo_terminal,
            commands::open_repo_vscode,
            commands::git_status_project,
            commands::git_fetch,
            commands::git_pull,
            commands::git_list_branches,
            commands::git_switch_branch,
            commands::list_dependencies,
            commands::set_repository_dependencies,
            commands::execute_project,
            commands::stop_all,
            commands::list_launch_history,
            commands::list_profiles,
            commands::create_profile,
            commands::update_profile,
            commands::delete_profile,
            commands::apply_profile,
            commands::export_profile,
            commands::import_profile,
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
