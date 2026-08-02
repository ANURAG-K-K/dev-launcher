//! Persistence layer (F13, ADR-0002 / D1; data-model.md).
//!
//! Owns the SQLite connection pool (`sqlx::SqlitePool`) and all reads/writes against the eight
//! canonical tables: projects, repositories, repository_dependencies, profiles,
//! profile_repositories, launch_history, launch_history_items, settings. No other module
//! touches the DB connection directly; the React frontend never opens SQLite.
//!
//! Migrations live under `src-tauri/migrations/` and run via `sqlx::migrate!()` at startup.
//!
//! All queries use the sqlx *runtime* query functions (`sqlx::query`, `sqlx::query_as`) rather
//! than the compile-time-checked `query!`/`query_as!` macros, since the latter require a live
//! `DATABASE_URL` at build time which this project does not provide.

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

/// A registered project root folder (data-model.md §3.1).
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub last_opened_at: String,
    pub last_profile_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

/// A discovered repository within a project (data-model.md §3.2).
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub path: String,
    pub package_manager: String,
    pub detected_script: Option<String>,
    pub command: Option<String>,
    pub args: Option<String>,
    pub env_file: Option<String>,
    pub enabled: i64,
    pub favorite: i64,
    /// When set, this repository launches in a visible interactive console window instead of
    /// piped background logs (trade-off: stdin/keyboard access vs. captured Logs output).
    pub visible_console: i64,
    pub removed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// Note: FromRow maps by the Rust field name to the DB column (both snake_case) — the serde
// rename_all="camelCase" only affects JSON serialization to the frontend, not FromRow.

/// Open (creating if missing) the SQLite pool with WAL journal mode and foreign keys enabled.
pub async fn init_pool(db_path: &std::path::Path) -> Result<SqlitePool, sqlx::Error> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| sqlx::Error::Io(e))?;
    }

    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);

    SqlitePoolOptions::new().connect_with(options).await
}

/// Run embedded migrations from `./migrations` (relative to `CARGO_MANIFEST_DIR`).
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| sqlx::Error::Migrate(Box::new(e)))
}

/// Insert the project if new, else update last_opened_at + updated_at. Keyed on
/// `root_path` (data-model.md §3.1, §3.2 upsert-on-rediscovery convention).
/// NOTE: `name` is deliberately excluded from the UPDATE clause so that user-defined renames
/// (via `rename_project`) survive reopening the project.
pub async fn upsert_project(
    pool: &SqlitePool,
    name: &str,
    root_path: &str,
) -> Result<Project, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        r#"
        INSERT INTO projects (name, root_path, last_opened_at, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(root_path) DO UPDATE SET
            last_opened_at = excluded.last_opened_at,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(name)
    .bind(root_path)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE root_path = ?")
        .bind(root_path)
        .fetch_one(pool)
        .await
}

/// Renames a project, returning the updated row.
pub async fn rename_project(pool: &SqlitePool, id: i64, name: &str) -> Result<Project, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE projects SET name = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Recent projects ordered by `last_opened_at` DESC.
pub async fn list_recent_projects(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<Project>, sqlx::Error> {
    sqlx::query_as::<_, Project>(
        "SELECT * FROM projects ORDER BY last_opened_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Insert the repository if new (using `default_enabled` for the initial `enabled`), else
/// update name/package_manager/detected_script and clear `removed_at` (un-remove on
/// rediscovery). On conflict, existing `enabled`, `command`, `args`, `env_file` (user
/// overrides) are preserved. Keyed on `(project_id, path)` (data-model.md §3.2).
pub async fn upsert_repository(
    pool: &SqlitePool,
    project_id: i64,
    name: &str,
    path: &str,
    package_manager: &str,
    detected_script: Option<&str>,
    default_enabled: bool,
) -> Result<Repository, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let enabled: i64 = if default_enabled { 1 } else { 0 };

    sqlx::query(
        r#"
        INSERT INTO repositories
            (project_id, name, path, package_manager, detected_script, enabled, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(project_id, path) DO UPDATE SET
            name = excluded.name,
            package_manager = excluded.package_manager,
            detected_script = excluded.detected_script,
            removed_at = NULL,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(project_id)
    .bind(name)
    .bind(path)
    .bind(package_manager)
    .bind(detected_script)
    .bind(enabled)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, Repository>(
        "SELECT * FROM repositories WHERE project_id = ? AND path = ?",
    )
    .bind(project_id)
    .bind(path)
    .fetch_one(pool)
    .await
}

/// Active (not soft-removed) repositories for a project, ordered by name.
pub async fn list_repositories(
    pool: &SqlitePool,
    project_id: i64,
) -> Result<Vec<Repository>, sqlx::Error> {
    sqlx::query_as::<_, Repository>(
        "SELECT * FROM repositories WHERE project_id = ? AND removed_at IS NULL ORDER BY favorite DESC, name",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
}

/// A launch run row (F13).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LaunchHistoryRow {
    pub id: i64,
    pub project_id: i64,
    pub profile_id: Option<i64>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
}

/// A per-repository row within a launch run (F13).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LaunchHistoryItemRow {
    pub id: i64,
    pub launch_history_id: i64,
    pub repository_id: i64,
    pub pid: Option<i64>,
    pub status: String,
    pub exit_code: Option<i64>,
    pub restart_count: i64,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
}

/// Open a launch-history run (status 'running'); returns its id.
pub async fn insert_launch_history(
    pool: &SqlitePool,
    project_id: i64,
    profile_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let res = sqlx::query(
        "INSERT INTO launch_history (project_id, profile_id, started_at, status) VALUES (?, ?, ?, 'running')",
    )
    .bind(project_id)
    .bind(profile_id)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Record a repository within a launch run; returns the new item id.
pub async fn insert_launch_history_item(
    pool: &SqlitePool,
    launch_history_id: i64,
    repository_id: i64,
    pid: Option<i64>,
    status: &str,
) -> Result<i64, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let res = sqlx::query(
        "INSERT INTO launch_history_items (launch_history_id, repository_id, pid, status, started_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(launch_history_id)
    .bind(repository_id)
    .bind(pid)
    .bind(status)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Mark a launch item as running with its PID (F13).
pub async fn update_launch_history_item_running(
    pool: &SqlitePool,
    item_id: i64,
    pid: Option<i64>,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE launch_history_items SET status = 'running', pid = ?, started_at = ? WHERE id = ?")
        .bind(pid)
        .bind(&now)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Mirror a process's terminal exit into its launch item (F13).
pub async fn update_launch_history_item_exit(
    pool: &SqlitePool,
    item_id: i64,
    status: &str,
    exit_code: Option<i64>,
    stopped_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE launch_history_items SET status = ?, exit_code = ?, stopped_at = ? WHERE id = ?")
        .bind(status)
        .bind(exit_code)
        .bind(stopped_at)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Sweep launches left `running` by a previous session (the app closed, so those OS processes are
/// gone). Mark their items `stopped` — not `crashed` — since a normal app quit stops them, and we
/// can't distinguish a clean quit from a crash here (F13, startup reconciliation).
pub async fn reconcile_stale_launches(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE launch_history_items SET status = 'stopped', stopped_at = ?
         WHERE status IN ('running', 'pending', 'restarting') AND stopped_at IS NULL",
    )
    .bind(&now)
    .execute(pool)
    .await?;
    // A run still 'running' at startup didn't finish cleanly; mark it completed (it dispatched)
    // rather than failed, to match the neutral 'stopped' items.
    sqlx::query("UPDATE launch_history SET status = 'completed', finished_at = ? WHERE status = 'running'")
        .bind(&now)
        .execute(pool)
        .await?;
    Ok(())
}

/// Close out a launch run with a terminal status.
pub async fn finish_launch_history(
    pool: &SqlitePool,
    id: i64,
    status: &str,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE launch_history SET status = ?, finished_at = ? WHERE id = ?")
        .bind(status)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Recent launch runs for a project, newest first.
pub async fn list_launch_history(
    pool: &SqlitePool,
    project_id: i64,
    limit: i64,
) -> Result<Vec<LaunchHistoryRow>, sqlx::Error> {
    sqlx::query_as::<_, LaunchHistoryRow>(
        "SELECT id, project_id, profile_id, started_at, finished_at, status
         FROM launch_history WHERE project_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(project_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// The repository items of a launch run.
pub async fn list_launch_history_items(
    pool: &SqlitePool,
    launch_history_id: i64,
) -> Result<Vec<LaunchHistoryItemRow>, sqlx::Error> {
    sqlx::query_as::<_, LaunchHistoryItemRow>(
        "SELECT * FROM launch_history_items WHERE launch_history_id = ? ORDER BY id",
    )
    .bind(launch_history_id)
    .fetch_all(pool)
    .await
}

/// Read a single settings value by key (F14).
pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

/// Upsert a single settings value by key (F14).
pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

/// A launch profile row (F6). Member repositories are stored separately in `profile_repositories`.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRow {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub launch_delay_ms: i64,
}

/// Create a profile, returning its new id (F6).
pub async fn create_profile(
    pool: &SqlitePool,
    project_id: i64,
    name: &str,
    launch_delay_ms: i64,
) -> Result<i64, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let res = sqlx::query(
        "INSERT INTO profiles (project_id, name, launch_delay_ms, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(project_id)
    .bind(name)
    .bind(launch_delay_ms)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Update a profile's name + launch delay.
pub async fn update_profile_meta(
    pool: &SqlitePool,
    profile_id: i64,
    name: &str,
    launch_delay_ms: i64,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE profiles SET name = ?, launch_delay_ms = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(launch_delay_ms)
        .bind(&now)
        .bind(profile_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a profile (cascades to its `profile_repositories`).
pub async fn delete_profile(pool: &SqlitePool, profile_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM profiles WHERE id = ?")
        .bind(profile_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// List a project's profiles (metadata only), ordered by name.
pub async fn list_profiles(
    pool: &SqlitePool,
    project_id: i64,
) -> Result<Vec<ProfileRow>, sqlx::Error> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT id, project_id, name, launch_delay_ms FROM profiles WHERE project_id = ? ORDER BY name",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
}

/// Fetch a single profile's metadata.
pub async fn get_profile(
    pool: &SqlitePool,
    profile_id: i64,
) -> Result<Option<ProfileRow>, sqlx::Error> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT id, project_id, name, launch_delay_ms FROM profiles WHERE id = ?",
    )
    .bind(profile_id)
    .fetch_optional(pool)
    .await
}

/// Replace a profile's member repositories (ordered enabled selection).
pub async fn set_profile_members(
    pool: &SqlitePool,
    profile_id: i64,
    repository_ids: &[i64],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM profile_repositories WHERE profile_id = ?")
        .bind(profile_id)
        .execute(&mut *tx)
        .await?;
    for (idx, rid) in repository_ids.iter().enumerate() {
        sqlx::query(
            "INSERT OR IGNORE INTO profile_repositories
                (profile_id, repository_id, enabled, launch_order)
             VALUES (?, ?, 1, ?)",
        )
        .bind(profile_id)
        .bind(rid)
        .bind(idx as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// A profile's member repository ids, in launch order.
pub async fn list_profile_members(
    pool: &SqlitePool,
    profile_id: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64,)>(
        "SELECT repository_id FROM profile_repositories WHERE profile_id = ? ORDER BY launch_order",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(r,)| r).collect())
}

/// Apply a profile: enable its member repos, disable all others in the project, and record it as
/// the project's last-used profile (F6 + restore-last-selection, F14).
pub async fn apply_profile(
    pool: &SqlitePool,
    project_id: i64,
    profile_id: i64,
    member_ids: &[i64],
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE repositories SET enabled = 0, updated_at = ? WHERE project_id = ?")
        .bind(&now)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
    for rid in member_ids {
        sqlx::query("UPDATE repositories SET enabled = 1, updated_at = ? WHERE id = ? AND project_id = ?")
            .bind(&now)
            .bind(rid)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE projects SET last_profile_id = ?, updated_at = ? WHERE id = ?")
        .bind(profile_id)
        .bind(&now)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// All dependency edges within a project, as `(repository_id, depends_on_repository_id)` (F8).
pub async fn list_dependencies(
    pool: &SqlitePool,
    project_id: i64,
) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT rd.repository_id, rd.depends_on_repository_id
         FROM repository_dependencies rd
         JOIN repositories r ON r.id = rd.repository_id
         WHERE r.project_id = ?",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
}

/// Replace a repository's `depends-on` edges with `depends_on` (F8). Self-edges and duplicates are
/// ignored by the `CHECK`/`UNIQUE` constraints via `INSERT OR IGNORE`. Cycle-checking is the
/// caller's responsibility (done before this write).
pub async fn set_repository_dependencies(
    pool: &SqlitePool,
    repository_id: i64,
    depends_on: &[i64],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM repository_dependencies WHERE repository_id = ?")
        .bind(repository_id)
        .execute(&mut *tx)
        .await?;
    for dep in depends_on {
        sqlx::query(
            "INSERT OR IGNORE INTO repository_dependencies (repository_id, depends_on_repository_id)
             VALUES (?, ?)",
        )
        .bind(repository_id)
        .bind(dep)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Fetch a single project by id, if it exists.
pub async fn get_project(pool: &SqlitePool, id: i64) -> Result<Option<Project>, sqlx::Error> {
    sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Fetch a single repository by id, if it exists.
pub async fn get_repository(pool: &SqlitePool, id: i64) -> Result<Option<Repository>, sqlx::Error> {
    sqlx::query_as::<_, Repository>("SELECT * FROM repositories WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Update a repository's command configuration (F5), returning the updated row.
/// `command`/`args`/`env_file` = None clears the override (repo reverts to the detected default).
pub async fn update_repository_config(
    pool: &SqlitePool,
    id: i64,
    package_manager: &str,
    command: Option<&str>,
    args: Option<&str>,
    env_file: Option<&str>,
) -> Result<Repository, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE repositories
         SET package_manager = ?, command = ?, args = ?, env_file = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(package_manager)
    .bind(command)
    .bind(args)
    .bind(env_file)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    sqlx::query_as::<_, Repository>("SELECT * FROM repositories WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Soft-remove a repository (F16): set `removed_at` so it drops out of the active list while its
/// `launch_history_items` are preserved. Rediscovery un-removes it (see `upsert_repository`).
pub async fn remove_repository(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE repositories SET removed_at = ?, updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Toggle a repository's `favorite` flag (R7), returning the updated row.
pub async fn set_repository_favorite(
    pool: &SqlitePool,
    id: i64,
    favorite: bool,
) -> Result<Repository, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE repositories SET favorite = ?, updated_at = ? WHERE id = ?")
        .bind(if favorite { 1 } else { 0 })
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query_as::<_, Repository>("SELECT * FROM repositories WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Toggle a repository's `visible_console` flag, returning the updated row.
pub async fn set_repository_visible_console(
    pool: &SqlitePool,
    id: i64,
    visible_console: bool,
) -> Result<Repository, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE repositories SET visible_console = ?, updated_at = ? WHERE id = ?")
        .bind(if visible_console { 1 } else { 0 })
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query_as::<_, Repository>("SELECT * FROM repositories WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Toggle a repository's `enabled` flag (F4), returning the updated row.
pub async fn set_repository_enabled(
    pool: &SqlitePool,
    id: i64,
    enabled: bool,
) -> Result<Repository, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE repositories SET enabled = ?, updated_at = ? WHERE id = ?")
        .bind(if enabled { 1 } else { 0 })
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query_as::<_, Repository>("SELECT * FROM repositories WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a fresh temp SQLite DB, migrated and ready to use. Returns the pool and the path
    /// so the test can delete the file afterwards.
    async fn setup_test_db() -> (SqlitePool, std::path::PathBuf) {
        let unique = format!(
            "mrl_test_{}_{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let path = std::env::temp_dir().join(unique);

        let pool = init_pool(&path).await.expect("init_pool failed");
        run_migrations(&pool).await.expect("run_migrations failed");

        (pool, path)
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        // WAL mode leaves -wal/-shm sidecar files; best-effort cleanup.
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[tokio::test]
    async fn upsert_project_inserts_then_updates_in_place() {
        let (pool, path) = setup_test_db().await;

        let first = upsert_project(&pool, "My Project", "/repos/my-project")
            .await
            .expect("first upsert failed");

        // Re-upsert with the same root_path: should update last_opened_at but preserve name.
        // (Name is only changeable via rename_project, not via upsert.)
        let second = upsert_project(&pool, "Different Name", "/repos/my-project")
            .await
            .expect("second upsert failed");

        assert_eq!(first.id, second.id);
        assert_eq!(second.name, "My Project", "name must not change on re-upsert, only via rename_project");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .expect("count query failed");
        assert_eq!(count, 1, "upsert must not duplicate rows for the same root_path");

        assert_ne!(
            first.last_opened_at, second.last_opened_at,
            "last_opened_at should refresh on re-upsert"
        );

        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn list_recent_projects_orders_by_last_opened_at_desc() {
        let (pool, path) = setup_test_db().await;

        upsert_project(&pool, "Older", "/repos/older")
            .await
            .expect("upsert older failed");

        // Ensure a distinct, later last_opened_at than the previous insert.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        upsert_project(&pool, "Newer", "/repos/newer")
            .await
            .expect("upsert newer failed");

        let recent = list_recent_projects(&pool, 10)
            .await
            .expect("list_recent_projects failed");

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].name, "Newer");
        assert_eq!(recent[1].name, "Older");

        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn upsert_repository_preserves_user_overrides_and_clears_removed_at() {
        let (pool, path) = setup_test_db().await;

        let project = upsert_project(&pool, "Proj", "/repos/proj")
            .await
            .expect("upsert project failed");

        let repo = upsert_repository(
            &pool,
            project.id,
            "api",
            "/repos/proj/api",
            "npm",
            Some("dev"),
            true,
        )
        .await
        .expect("initial upsert_repository failed");

        assert_eq!(repo.enabled, 1);
        assert!(repo.removed_at.is_none());

        // Simulate a user manually disabling the repo and soft-removing it.
        sqlx::query("UPDATE repositories SET enabled = 0, removed_at = ? WHERE id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(repo.id)
            .execute(&pool)
            .await
            .expect("manual update failed");

        // Rediscovery: upsert again with a different detected_script/package_manager.
        let rediscovered = upsert_repository(
            &pool,
            project.id,
            "api",
            "/repos/proj/api",
            "pnpm",
            Some("start"),
            true, // default_enabled must be ignored on update path
        )
        .await
        .expect("re-upsert failed");

        assert_eq!(rediscovered.id, repo.id);
        assert_eq!(rediscovered.package_manager, "pnpm");
        assert_eq!(rediscovered.detected_script.as_deref(), Some("start"));
        assert_eq!(
            rediscovered.enabled, 0,
            "enabled must be preserved (user override), not reset by default_enabled"
        );
        assert!(
            rediscovered.removed_at.is_none(),
            "removed_at must be cleared on rediscovery"
        );

        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn list_repositories_excludes_soft_removed_and_orders_by_name() {
        let (pool, path) = setup_test_db().await;

        let project = upsert_project(&pool, "Proj", "/repos/proj2")
            .await
            .expect("upsert project failed");

        let repo_b = upsert_repository(
            &pool,
            project.id,
            "b-repo",
            "/repos/proj2/b",
            "npm",
            None,
            true,
        )
        .await
        .expect("upsert b failed");

        upsert_repository(
            &pool,
            project.id,
            "a-repo",
            "/repos/proj2/a",
            "npm",
            None,
            true,
        )
        .await
        .expect("upsert a failed");

        let removed = upsert_repository(
            &pool,
            project.id,
            "c-repo",
            "/repos/proj2/c",
            "npm",
            None,
            true,
        )
        .await
        .expect("upsert c failed");

        // Soft-remove repo "c-repo" directly.
        sqlx::query("UPDATE repositories SET removed_at = ? WHERE id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(removed.id)
            .execute(&pool)
            .await
            .expect("soft-remove failed");

        let active = list_repositories(&pool, project.id)
            .await
            .expect("list_repositories failed");

        assert_eq!(active.len(), 2);
        assert_eq!(active[0].name, "a-repo");
        assert_eq!(active[1].name, "b-repo");
        assert!(active.iter().all(|r| r.id != removed.id));
        assert!(active.iter().any(|r| r.id == repo_b.id));

        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn rename_project_updates_name_only() {
        let (pool, _path) = setup_test_db().await;
        let created = upsert_project(&pool, "Original", "/repos/x")
            .await
            .expect("upsert failed");

        let renamed = rename_project(&pool, created.id, "Renamed")
            .await
            .expect("rename failed");

        assert_eq!(renamed.id, created.id);
        assert_eq!(renamed.name, "Renamed");
        assert_eq!(renamed.root_path, created.root_path);
        assert_eq!(renamed.created_at, created.created_at);
        assert_ne!(renamed.updated_at, created.updated_at, "updated_at should refresh on rename");
    }

    #[tokio::test]
    async fn upsert_project_does_not_revert_a_rename() {
        let (pool, _path) = setup_test_db().await;
        let created = upsert_project(&pool, "folder-basename", "/repos/y")
            .await
            .expect("upsert failed");
        rename_project(&pool, created.id, "My Custom Name")
            .await
            .expect("rename failed");

        // Simulates reopening the project: open_project always re-derives the name from the
        // folder basename and calls upsert_project again with it.
        let reopened = upsert_project(&pool, "folder-basename", "/repos/y")
            .await
            .expect("re-upsert failed");

        assert_eq!(reopened.id, created.id);
        assert_eq!(
            reopened.name, "My Custom Name",
            "a rename must survive reopening the project — this is the bug this task fixes"
        );
    }
}
