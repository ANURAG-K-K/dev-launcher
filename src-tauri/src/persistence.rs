//! Persistence layer (F13, ADR-0002 / D1; data-model.md).
//!
//! Owns the SQLite connection pool (`sqlx::SqlitePool`) and all reads/writes against the eight
//! canonical tables: projects, repositories, repository_dependencies, profiles,
//! profile_repositories, launch_history, launch_history_items, settings. No other module
//! touches the DB connection directly; the React frontend never opens SQLite.
//!
//! Migrations live under `src-tauri/migrations/` and run via `sqlx::migrate!()` at startup.
//!
//! TODO: implement the connection pool setup (WAL mode) and per-table repository functions.
