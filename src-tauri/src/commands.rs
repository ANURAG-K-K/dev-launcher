//! Tauri command IPC surface (architecture §5). First slice: project discovery.
//!
//! Implemented: `open_project`, `scan_repositories` (refresh), `list_recent_projects`.
//! Remaining commands (start/stop/restart, profiles, logs, settings) land in later slices.

use tauri::State;

use crate::error::AppError;
use crate::{persistence, scanner, AppState};

/// Response for `open_project` / `scan_repositories`: the project plus its active repositories.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWithRepos {
    pub project: persistence::Project,
    pub repositories: Vec<persistence::Repository>,
}

/// Discover + persist repositories under a project root, registering/refreshing the project.
/// (F1 open project + F2/F3 discovery.)
#[tauri::command]
pub async fn open_project(
    state: State<'_, AppState>,
    root_path: String,
) -> Result<ProjectWithRepos, AppError> {
    let root = std::path::Path::new(&root_path);
    // NFR-11: validate the path is an existing directory before persisting anything.
    if !root.is_dir() {
        return Err(AppError::Scan(format!(
            "not a directory: {root_path}"
        )));
    }

    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root_path.clone());

    let project = persistence::upsert_project(&state.pool, &name, &root_path)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;

    let repositories = discover_and_persist(&state.pool, project.id, root).await?;
    Ok(ProjectWithRepos {
        project,
        repositories,
    })
}

/// Re-scan an already-open project (manual Refresh, F2/D3).
#[tauri::command]
pub async fn scan_repositories(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<ProjectWithRepos, AppError> {
    let project = persistence::get_project(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("project {project_id} not found")))?;

    let root = std::path::Path::new(&project.root_path);
    let repositories = discover_and_persist(&state.pool, project.id, root).await?;
    Ok(ProjectWithRepos {
        project,
        repositories,
    })
}

/// Recent projects for the Home screen (F1).
#[tauri::command]
pub async fn list_recent_projects(
    state: State<'_, AppState>,
) -> Result<Vec<persistence::Project>, AppError> {
    persistence::list_recent_projects(&state.pool, 20)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Shared helper: scan the root, upsert each discovered repo, return the active list.
async fn discover_and_persist(
    pool: &sqlx::SqlitePool,
    project_id: i64,
    root: &std::path::Path,
) -> Result<Vec<persistence::Repository>, AppError> {
    let discovered = scanner::scan_project_root(root).map_err(|e| AppError::Scan(e.to_string()))?;

    for repo in &discovered {
        // New repos default enabled only when a startup script was detected (design.md §1.3).
        let default_enabled = repo.detected_script.is_some();
        persistence::upsert_repository(
            pool,
            project_id,
            &repo.name,
            &repo.path,
            repo.package_manager.as_str(),
            repo.detected_script.as_deref(),
            default_enabled,
        )
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    }

    persistence::list_repositories(pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}
