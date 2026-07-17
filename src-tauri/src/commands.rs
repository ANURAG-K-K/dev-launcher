//! Tauri command/event IPC surface (architecture §5) — the sole boundary between the React
//! frontend and the Rust backend. Commands are request/response; events (`repo_log`,
//! `repo_status_changed`, `launch_progress`, `launch_completed`, `scan_progress`) are push.
//!
//! Planned commands: open_project, list_recent_projects, scan_repositories,
//! update_repository_config, set_repository_dependencies, create/update/delete/list_profile(s),
//! start_repo, stop_repo, restart_repo, start_profile, stop_all, restart_all, get_logs,
//! open_repo_folder, open_repo_terminal, get_settings, update_settings, get_launch_history.
//!
//! TODO: implement `#[tauri::command]` handlers and register them in `lib.rs`.
