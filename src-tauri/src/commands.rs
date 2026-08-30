//! Tauri command IPC surface (architecture §5). First slice: project discovery.
//!
//! Implemented: `open_project`, `scan_repositories` (refresh), `list_recent_projects`.
//! Remaining commands (start/stop/restart, profiles, logs, settings) land in later slices.

use tauri::State;

use crate::error::AppError;
use crate::process_manager::{LaunchSpec, StatusUpdate};
use crate::{launcher, persistence, scanner, AppState};

/// Response for `open_project` / `scan_repositories`: the project plus its active repositories.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWithRepos {
    pub project: persistence::Project,
    pub repositories: Vec<persistence::Repository>,
}

/// Response for `refresh_repositories`: the active repository list plus which of them
/// couldn't be found on disk, without mutating anything for a repo that's simply missing.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    pub project: persistence::Project,
    pub repositories: Vec<persistence::Repository>,
    pub missing_repository_ids: Vec<i64>,
    pub root_missing: bool,
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

/// Trims and validates a project name is non-empty. Returns the trimmed name.
fn validate_project_name(name: &str) -> Result<&str, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::Persist("project name cannot be empty".into()));
    }
    Ok(trimmed)
}

/// Renames a project (sidebar/Home rename UI).
#[tauri::command]
pub async fn rename_project(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
) -> Result<persistence::Project, AppError> {
    let trimmed = validate_project_name(&name)?;
    persistence::rename_project(&state.pool, project_id, trimmed)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
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

/// Lightweight check-and-refresh for the already-listed (active, non-removed) repositories of a
/// project: verifies each one's folder + `package.json` still exist and re-detects its package
/// manager/script if so, but - unlike `scan_repositories` ("Re-scan") - never discovers new
/// repos and never un-removes a repo the user deliberately removed. A repo whose folder can't be
/// found is reported via `missing_repository_ids` without touching its row at all, so a
/// temporary/misdetected miss can't silently corrupt its config. If the project root itself is
/// gone, per-repo checks are skipped entirely (`root_missing: true`) rather than reporting every
/// repo as individually missing.
#[tauri::command]
pub async fn refresh_repositories(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<RefreshResult, AppError> {
    let project = persistence::get_project(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("project {project_id} not found")))?;

    let root_missing = !std::path::Path::new(&project.root_path).is_dir();
    let mut repositories = persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let mut missing_repository_ids = Vec::new();

    if !root_missing {
        for repo in repositories.iter_mut() {
            let dir = std::path::Path::new(&repo.path);
            match scanner::classify_repo_dir(dir, Some(&repo.name)) {
                Some(discovered) => {
                    if discovered.package_manager.as_str() != repo.package_manager
                        || discovered.detected_script != repo.detected_script
                    {
                        *repo = persistence::refresh_repository_detection(
                            &state.pool,
                            repo.id,
                            discovered.package_manager.as_str(),
                            discovered.detected_script.as_deref(),
                        )
                        .await
                        .map_err(|e| AppError::Persist(e.to_string()))?;
                    }
                }
                None => missing_repository_ids.push(repo.id),
            }
        }
    }

    Ok(RefreshResult {
        project,
        repositories,
        missing_repository_ids,
        root_missing,
    })
}

/// Deletes a project and everything under it (repositories, dependencies, profiles, launch
/// history - all via existing ON DELETE CASCADE foreign keys). Rejects the deletion if any of
/// the project's repositories are currently running or starting, checked against the live
/// process tracker rather than only the database, so a stale frontend status cache can't lead
/// to an orphaned OS process.
#[tauri::command]
pub async fn delete_project(state: State<'_, AppState>, project_id: i64) -> Result<(), AppError> {
    let repos = persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    if repos.iter().any(|r| state.process_manager.is_running(r.id)) {
        return Err(AppError::Persist(
            "stop all repositories in this project before deleting".into(),
        ));
    }
    persistence::delete_project(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Count of currently-*running* (not merely enabled) services per project, for the sidebar's
/// per-project running indicator. Read-only and derived entirely from existing state (the
/// service list + the live process tracker) - no new persisted "running count" data. A project
/// with zero repos, or an unknown project id, simply gets `0` rather than an error, since the
/// sidebar calls this for every recent project in one batch and one stale/missing id shouldn't
/// fail the whole batch.
///
/// No automated test: like `delete_project`'s running-repo guard, this depends on
/// `ProcessManager::is_running`, which needs a real Tauri `AppHandle` that can't be constructed
/// in a unit test - the same accepted gap, verified manually instead.
#[tauri::command]
pub async fn project_running_counts(
    state: State<'_, AppState>,
    project_ids: Vec<i64>,
) -> Result<std::collections::HashMap<i64, i64>, AppError> {
    let mut counts = std::collections::HashMap::new();
    for project_id in project_ids {
        let repos = persistence::list_repositories(&state.pool, project_id)
            .await
            .map_err(|e| AppError::Persist(e.to_string()))?;
        let running = repos
            .iter()
            .filter(|r| state.process_manager.is_running(r.id))
            .count() as i64;
        counts.insert(project_id, running);
    }
    Ok(counts)
}

/// Repoints a project at a new root folder. Any repository that still exists at the same
/// relative path under the new root is updated in place (id + user overrides preserved);
/// anything else is picked up by the normal scan that follows. `root_path` and any remapped
/// repos land together in one transaction (`apply_project_remap`), so a mid-failure never leaves
/// them pointing at different locations.
#[tauri::command]
pub async fn update_project_path(
    state: State<'_, AppState>,
    project_id: i64,
    new_root_path: String,
) -> Result<ProjectWithRepos, AppError> {
    let new_root = std::path::Path::new(&new_root_path);
    if !new_root.is_dir() {
        return Err(AppError::Scan(format!("not a directory: {new_root_path}")));
    }

    let project = persistence::get_project(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("project {project_id} not found")))?;
    let old_root = std::path::Path::new(&project.root_path);

    let existing = persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let remap = compute_repo_remap(old_root, new_root, &existing);

    persistence::apply_project_remap(&state.pool, project_id, &new_root_path, &remap)
        .await
        .map_err(|e| {
            if e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false) {
                AppError::Persist("another project already uses this path".into())
            } else {
                AppError::Persist(e.to_string())
            }
        })?;

    let repositories = discover_and_persist(&state.pool, project_id, new_root).await?;
    let project = persistence::get_project(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("project {project_id} not found")))?;
    Ok(ProjectWithRepos { project, repositories })
}

/// Manually adds a single repository by path, for cases the auto-scan doesn't reach (feedback
/// #8: nested repos more than one level deep, or any other layout the heuristic misses).
/// Rejects a directory without a `package.json`; otherwise behaves like a one-repo scan.
#[tauri::command]
pub async fn add_repository_manual(
    state: State<'_, AppState>,
    project_id: i64,
    path: String,
) -> Result<Vec<persistence::Repository>, AppError> {
    let dir = std::path::Path::new(&path);
    if !dir.is_dir() {
        return Err(AppError::Scan(format!("not a directory: {path}")));
    }
    let repo = scanner::classify_repo_dir(dir, None)
        .ok_or_else(|| AppError::Scan(format!("no package.json found in: {path}")))?;

    let default_enabled = repo.detected_script.is_some();
    persistence::upsert_repository(
        &state.pool,
        project_id,
        &repo.name,
        &repo.path,
        repo.package_manager.as_str(),
        repo.detected_script.as_deref(),
        default_enabled,
    )
    .await
    .map_err(|e| AppError::Persist(e.to_string()))?;

    persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Repoints a single repository at a new folder. Rejects outright if the folder has no
/// `package.json` (no "save with a warning" path). Name is taken from the new folder (kept in
/// sync with what it points to, matching how auto-scan already names repos); package manager and
/// detected script are refreshed; every user override on the row is left untouched.
#[tauri::command]
pub async fn update_repository_path(
    state: State<'_, AppState>,
    repository_id: i64,
    new_path: String,
) -> Result<persistence::Repository, AppError> {
    persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("repository {repository_id} not found")))?;

    let dir = std::path::Path::new(&new_path);
    if !dir.is_dir() {
        return Err(AppError::Scan(format!("not a directory: {new_path}")));
    }
    let discovered = scanner::classify_repo_dir(dir, None)
        .ok_or_else(|| AppError::Scan(format!("no package.json found in: {new_path}")))?;

    persistence::update_repository_path(
        &state.pool,
        repository_id,
        &discovered.path,
        &discovered.name,
        discovered.package_manager.as_str(),
        discovered.detected_script.as_deref(),
    )
    .await
    .map_err(|e| {
        if e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false) {
            AppError::Persist("another repository in this project already uses this path".into())
        } else {
            AppError::Persist(e.to_string())
        }
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

/// For each existing repo, checks whether a `package.json` still exists at the same path
/// relative to `new_root` as the repo currently has relative to `old_root`. Repos that don't
/// match (moved away, or the new root has a different layout) are simply absent from the
/// result - left for the caller (and a normal Refresh) to leave untouched.
///
/// Caveat on Windows: `strip_prefix` does exact component comparison (case-sensitive apart from
/// the drive letter), so if `old_root`/`new_root` ever differ from a repo's stored path in case
/// or short-vs-long (8.3) form, that repo will silently fail to match and be excluded - which,
/// combined with the subsequent full rescan, produces duplicate repo rows at the new location
/// rather than a clean no-op.
fn compute_repo_remap(
    old_root: &std::path::Path,
    new_root: &std::path::Path,
    repos: &[persistence::Repository],
) -> Vec<persistence::RepoRemap> {
    let mut out = Vec::new();
    for repo in repos {
        let repo_path = std::path::Path::new(&repo.path);
        let Ok(relative) = repo_path.strip_prefix(old_root) else {
            continue;
        };
        let candidate = new_root.join(relative);
        if let Some(discovered) = scanner::classify_repo_dir(&candidate, Some(&repo.name)) {
            out.push(persistence::RepoRemap {
                repository_id: repo.id,
                path: discovered.path,
                package_manager: discovered.package_manager.as_str().to_string(),
                detected_script: discovered.detected_script,
            });
        }
    }
    out
}

// ── Dependencies & sequential launch (F7/F8) ───────────────────────────────

/// A dependency edge: `repository_id` depends on `depends_on_repository_id`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdge {
    pub repository_id: i64,
    pub depends_on_repository_id: i64,
}

/// Outcome of an "Execute All" run.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteResult {
    pub started: Vec<i64>,
    pub skipped: Vec<i64>,
}

/// All dependency edges within a project (F8).
#[tauri::command]
pub async fn list_dependencies(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<DependencyEdge>, AppError> {
    let edges = persistence::list_dependencies(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    Ok(edges
        .into_iter()
        .map(|(repository_id, depends_on_repository_id)| DependencyEdge {
            repository_id,
            depends_on_repository_id,
        })
        .collect())
}

/// Replace a repository's `depends-on` set, rejecting changes that introduce a cycle (F8/FR-18).
#[tauri::command]
pub async fn set_repository_dependencies(
    state: State<'_, AppState>,
    repository_id: i64,
    depends_on: Vec<i64>,
) -> Result<(), AppError> {
    let repo = persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("repository {repository_id} not found")))?;

    let repos = persistence::list_repositories(&state.pool, repo.project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let nodes: Vec<i64> = repos.iter().map(|r| r.id).collect();

    // Build the would-be edge set (existing edges minus this repo's, plus the proposed ones).
    let mut edges: Vec<(i64, i64)> = persistence::list_dependencies(&state.pool, repo.project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .into_iter()
        .filter(|(r, _)| *r != repository_id)
        .collect();
    for dep in &depends_on {
        if *dep != repository_id {
            edges.push((repository_id, *dep));
        }
    }

    if let Err(cyclic) = launcher::topological_order(&nodes, &edges) {
        return Err(AppError::Cycle(cycle_names(&repos, &cyclic)));
    }

    persistence::set_repository_dependencies(&state.pool, repository_id, &depends_on)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Execute all enabled repositories in a project sequentially (F7), honoring dependency order
/// (F8). A repo whose dependency is disabled/unlaunchable is skipped (policy A4: block + report);
/// a dependency cycle aborts the whole launch before anything starts.
#[tauri::command]
pub async fn execute_project(
    state: State<'_, AppState>,
    project_id: i64,
    launch_delay_ms: u64,
) -> Result<ExecuteResult, AppError> {
    let repos = persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let edges = persistence::list_dependencies(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;

    let enabled_ids: Vec<i64> = repos.iter().filter(|r| r.enabled == 1).map(|r| r.id).collect();

    // Start from enabled repos that actually have a launch command.
    let mut runnable: std::collections::HashSet<i64> = repos
        .iter()
        .filter(|r| r.enabled == 1 && (r.command.is_some() || r.detected_script.is_some()))
        .map(|r| r.id)
        .collect();

    // Block-on-unmet-dependency (A4): drop any repo that depends on something outside the runnable
    // set, repeating until stable (so transitive blockers propagate).
    loop {
        let to_remove: Vec<i64> = runnable
            .iter()
            .copied()
            .filter(|&r| edges.iter().any(|&(dep, on)| dep == r && !runnable.contains(&on)))
            .collect();
        if to_remove.is_empty() {
            break;
        }
        for r in to_remove {
            runnable.remove(&r);
        }
    }

    // Stable order (project repo order) restricted to the runnable set.
    let runnable_nodes: Vec<i64> = repos
        .iter()
        .map(|r| r.id)
        .filter(|id| runnable.contains(id))
        .collect();
    let runnable_edges: Vec<(i64, i64)> = edges
        .iter()
        .copied()
        .filter(|&(a, b)| runnable.contains(&a) && runnable.contains(&b))
        .collect();

    let order = launcher::topological_order(&runnable_nodes, &runnable_edges)
        .map_err(|cyclic| AppError::Cycle(cycle_names(&repos, &cyclic)))?;

    // Record the run in launch history (F13). profile_id = the project's last-applied profile.
    let profile_id = persistence::get_project(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .and_then(|p| p.last_profile_id);
    let history_id = persistence::insert_launch_history(&state.pool, project_id, profile_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;

    let mut started = Vec::new();
    for (i, id) in order.iter().enumerate() {
        let spec = launch_spec_for(&state, *id).await?;
        // Create the history item first so the process manager can mirror this repo's exit into it.
        let item_id = persistence::insert_launch_history_item(&state.pool, history_id, *id, None, "pending")
            .await
            .ok();
        match state.process_manager.start(*id, spec, item_id).await {
            Ok(update) => {
                started.push(*id);
                if let Some(iid) = item_id {
                    let _ = persistence::update_launch_history_item_running(
                        &state.pool,
                        iid,
                        update.pid.map(|p| p as i64),
                    )
                    .await;
                }
            }
            Err(_) => {
                if let Some(iid) = item_id {
                    let now = chrono::Utc::now().to_rfc3339();
                    let _ = persistence::update_launch_history_item_exit(
                        &state.pool, iid, "crashed", None, &now,
                    )
                    .await;
                }
            }
        }
        if i + 1 < order.len() {
            tokio::time::sleep(std::time::Duration::from_millis(launch_delay_ms)).await;
        }
    }

    let final_status = if started.is_empty() { "failed" } else { "completed" };
    let _ = persistence::finish_launch_history(&state.pool, history_id, final_status).await;

    let skipped: Vec<i64> = enabled_ids
        .into_iter()
        .filter(|id| !started.contains(id))
        .collect();
    Ok(ExecuteResult { started, skipped })
}

/// Stop every running repository in a project (F9 - Stop All). Repos that aren't running are
/// ignored.
#[tauri::command]
pub async fn stop_all(state: State<'_, AppState>, project_id: i64) -> Result<(), AppError> {
    let repos = persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    for repo in repos {
        // Ignore "not running" errors - we only care that nothing is left running.
        let _ = state.process_manager.stop(repo.id);
    }
    Ok(())
}

/// A repository within a launch record (F13, shaped for the UI).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchItemRecord {
    pub repository_name: String,
    pub status: String,
    pub pid: Option<i64>,
    pub exit_code: Option<i64>,
    pub stopped_at: Option<String>,
}

/// A launch run with its repositories resolved to names (F13).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRecord {
    pub id: i64,
    pub profile_name: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub items: Vec<LaunchItemRecord>,
}

/// Recent launch runs for a project (F13).
#[tauri::command]
pub async fn list_launch_history(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<LaunchRecord>, AppError> {
    let rows = persistence::list_launch_history(&state.pool, project_id, 25)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let profile_name = match row.profile_id {
            Some(pid) => persistence::get_profile(&state.pool, pid)
                .await
                .map_err(|e| AppError::Persist(e.to_string()))?
                .map(|p| p.name),
            None => None,
        };
        let item_rows = persistence::list_launch_history_items(&state.pool, row.id)
            .await
            .map_err(|e| AppError::Persist(e.to_string()))?;
        let mut items = Vec::with_capacity(item_rows.len());
        for it in item_rows {
            let repository_name = persistence::get_repository(&state.pool, it.repository_id)
                .await
                .map_err(|e| AppError::Persist(e.to_string()))?
                .map(|r| r.name)
                .unwrap_or_else(|| format!("#{}", it.repository_id));
            items.push(LaunchItemRecord {
                repository_name,
                status: it.status,
                pid: it.pid,
                exit_code: it.exit_code,
                stopped_at: it.stopped_at,
            });
        }
        out.push(LaunchRecord {
            id: row.id,
            profile_name,
            started_at: row.started_at,
            finished_at: row.finished_at,
            status: row.status,
            items,
        });
    }
    Ok(out)
}

/// Format a cycle's repo ids as a human-readable "name, name" list for an error message.
fn cycle_names(repos: &[persistence::Repository], cyclic: &[i64]) -> String {
    let names: Vec<String> = cyclic
        .iter()
        .map(|id| {
            repos
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.name.clone())
                .unwrap_or_else(|| format!("#{id}"))
        })
        .collect();
    format!("dependency cycle involving: {}", names.join(", "))
}

// ── Repo actions: open folder / open terminal (F12) ─────────────────────────

/// Open the repository's working directory in the system file explorer (F12).
#[tauri::command]
pub async fn open_repo_folder(
    state: State<'_, AppState>,
    repository_id: i64,
) -> Result<(), AppError> {
    let repo = persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Launch(format!("repository {repository_id} not found")))?;
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&repo.path)
            .spawn()
            .map_err(|e| AppError::Launch(format!("failed to open folder: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = repo;
        Err(AppError::Launch("open folder is Windows-only in v1".into()))
    }
}

/// Lists `.env`-style files sitting directly in a service's own folder (not recursive), sorted,
/// so the Env file field can offer them as a dropdown instead of requiring the user to type a
/// path. A plain `&SqlitePool` function for the same testability reason as
/// `export_profile_impl`/`import_profile_impl` - it only needs the repository's path, never the
/// process manager.
async fn list_env_files_impl(pool: &sqlx::SqlitePool, repository_id: i64) -> Result<Vec<String>, AppError> {
    let repo = persistence::get_repository(pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("repository {repository_id} not found")))?;

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&repo.path) {
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".env" || name.starts_with(".env.") {
                files.push(name);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Lists `.env`-style files in a service's own folder, for the Env file dropdown (feedback:
/// env file should be selectable, not typed).
#[tauri::command]
pub async fn list_env_files(state: State<'_, AppState>, repository_id: i64) -> Result<Vec<String>, AppError> {
    list_env_files_impl(&state.pool, repository_id).await
}

/// Open an external terminal at the repository's working directory (F12). An in-app
/// "integrated" terminal is deferred past v1 (R6) - this always opens an external window, in
/// the shell chosen by `Settings.terminal_shell`.
///
/// When Windows Terminal is installed, this opens as a new tab in the existing `wt` window
/// instead of a brand-new separate window (feedback #4) - `wt.exe` is only ever the UI shell for
/// this blank interactive terminal; it is never tracked or added to the app's managed process
/// tree the way a repo's dev-server process is. Falls back to the original behavior (a plain new
/// `cmd`/`powershell` window) when Windows Terminal isn't found.
#[tauri::command]
pub async fn open_repo_terminal(
    state: State<'_, AppState>,
    repository_id: i64,
) -> Result<(), AppError> {
    let repo = persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Launch(format!("repository {repository_id} not found")))?;

    #[cfg(windows)]
    {
        let settings = load_settings(&state.pool).await?;
        let shell = shell_program(&settings.terminal_shell);

        if windows_terminal_available() {
            std::process::Command::new("wt")
                .args(wt_launch_args(shell, &repo.path))
                .spawn()
                .map_err(|e| AppError::Launch(format!("failed to open terminal: {e}")))?;
            return Ok(());
        }

        let mut args = vec!["/C", "start", "", shell];
        if shell == "powershell" {
            // Same execution-policy bypass as windows_launch_program_args - otherwise the
            // first npm/pnpm/yarn/bun command typed in this window fails to load its .ps1 shim.
            args.extend(["-ExecutionPolicy", "Bypass"]);
        }
        std::process::Command::new("cmd")
            .args(&args)
            .current_dir(&repo.path)
            .spawn()
            .map_err(|e| AppError::Launch(format!("failed to open terminal: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = repo;
        Err(AppError::Launch("open terminal is Windows-only in v1".into()))
    }
}

/// Opens the repository folder in VS Code. Windows-only in v1, matching
/// `open_repo_folder`/`open_repo_terminal`. `code` on Windows is a `.cmd` shim (like
/// npm/pnpm), so it needs `cmd /C` wrapping - CREATE_NO_WINDOW keeps that wrapper invisible;
/// VS Code opens its own window regardless.
#[tauri::command]
pub async fn open_repo_vscode(
    state: State<'_, AppState>,
    repository_id: i64,
) -> Result<(), AppError> {
    let repo = persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Launch(format!("repository {repository_id} not found")))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/C", "code", &repo.path])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map_err(|e| {
                AppError::Launch(format!("failed to open VS Code: {e} (is 'code' on your PATH?)"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = repo;
        Err(AppError::Launch("open with VS Code is Windows-only in v1".into()))
    }
}

// ── Git integration (R5) ────────────────────────────────────────────────────

/// Per-repository git status (R5). Only returned for repos that are git working trees.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoGitStatus {
    pub repository_id: i64,
    pub branch: String,
    pub dirty: bool,
    pub ahead: i64,
    pub behind: i64,
}

/// Git status for every git-tracked repository in a project (R5). Non-git repos are omitted.
/// Serves from the process-local cache (`GIT_STATUS_CACHE_TTL`) where possible; only cache
/// misses spawn a git subprocess, bounded by a semaphore so a large project doesn't fire one
/// git process per repo simultaneously.
#[tauri::command]
pub async fn git_status_project(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<RepoGitStatus>, AppError> {
    let repos = persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;

    let cache = state.git_status_cache.clone();
    let permits = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(4);
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(permits));

    let mut handles = Vec::with_capacity(repos.len());
    for repo in repos {
        let cache = cache.clone();
        let semaphore = semaphore.clone();
        handles.push(tokio::spawn(async move {
            // "canonical" here means the repository's DB path used verbatim as the cache key -
            // it is NOT actually passed through Path::canonicalize() (no symlink/`..`/case resolution).
            let canonical_path = std::path::PathBuf::from(&repo.path);
            // Concurrent invocations of git_status_project (e.g. rapid UI tab-switching) can both
            // miss the cache and spawn git for the same repo. This is intentionally tolerated because
            // the duplicate work is flash-free and non-blocking (post the CREATE_NO_WINDOW/spawn_blocking
            // fixes), and adding single-flight dedup would be more complexity than this bug warrants.
            if let Some(cached) = cache.get(repo.id, &canonical_path, crate::git_status_cache::GIT_STATUS_CACHE_TTL) {
                return Some(cached);
            }

            let _permit = semaphore.acquire_owned().await.ok()?;
            let path = repo.path.clone();
            let repo_id = repo.id;
            // Only the blocking git subprocess calls run inside spawn_blocking; the cache
            // check and semaphore acquisition above stay in plain async code.
            let status = tokio::task::spawn_blocking(move || git_status_for(&path, repo_id))
                .await
                .ok()
                .flatten()?;

            cache.insert(repo_id, canonical_path, status.clone());
            Some(status)
        }));
    }

    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        // A repo whose git status check panics or fails is intentionally omitted from the results,
        // consistent with the existing "non-git repos are omitted" contract - not a bug, a deliberate choice.
        if let Ok(Some(status)) = handle.await {
            out.push(status);
        }
    }
    Ok(out)
}

/// A branch available to switch to (R-branch-switch). `is_current` is never true for more than
/// one entry, and is never true at all while the repo is in detached-HEAD state (HEAD matches
/// no branch name in that case).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchEntry {
    pub name: String,
    pub is_current: bool,
}

const COMMON_BRANCH_ORDER: &[&str] = &["main", "master", "develop"];

/// Sort key: current branch first, then the fixed common-branch order, then alphabetical.
fn branch_sort_key(name: &str, current: &str) -> (u8, usize, String) {
    if name == current {
        (0, 0, String::new())
    } else if let Some(idx) = COMMON_BRANCH_ORDER.iter().position(|c| *c == name) {
        (1, idx, String::new())
    } else {
        (2, 0, name.to_string())
    }
}

/// Local branches ∪ remote-only branches (by plain name, `origin/` stripped, `origin/HEAD`
/// excluded), deduplicated, sorted per `branch_sort_key`. Assumes a single `origin` remote.
fn git_list_branches_impl(path: &str) -> Result<Vec<BranchEntry>, AppError> {
    let current = git_command(path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let local: Vec<String> = {
        let out = git_command(path)
            .args(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .output()
            .map_err(|e| AppError::Launch(format!("git for-each-ref failed: {e}")))?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };

    let remote: Vec<String> = {
        let out = git_command(path)
            .args(["for-each-ref", "--format=%(refname:short)", "refs/remotes"])
            .output()
            .map_err(|e| AppError::Launch(format!("git for-each-ref failed: {e}")))?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.ends_with("/HEAD"))
            .filter_map(|l| l.split_once('/').map(|(_, name)| name.to_string()))
            .collect()
    };

    let mut names = local;
    for r in remote {
        if !names.contains(&r) {
            names.push(r);
        }
    }

    names.sort_by(|a, b| branch_sort_key(a, &current).cmp(&branch_sort_key(b, &current)));

    Ok(names
        .into_iter()
        .map(|name| {
            let is_current = name == current;
            BranchEntry { name, is_current }
        })
        .collect())
}

#[tauri::command]
pub async fn git_list_branches(
    state: State<'_, AppState>,
    repository_id: i64,
) -> Result<Vec<BranchEntry>, AppError> {
    let path = git_repo_path(&state, repository_id).await?;
    git_list_branches_impl(&path)
}

/// Builds a `git -C <path>` command with console-window suppression on Windows (every git
/// subprocess in this app must be spawned through this helper - see spec 2026-07-31).
fn git_command(path: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// True if `path`'s working tree has any uncommitted changes (tracked or untracked).
fn git_is_dirty(path: &str) -> bool {
    git_command(path)
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Resolves `refs/stash` to a commit hash, or `None` if the stash list is empty. Comparing this
/// before/after a `git stash push` is the reliable way to tell whether a stash was actually
/// created (vs. `git status --porcelain` dirtiness, which includes untracked files that a
/// non-`-u` stash push silently skips).
fn stash_ref(path: &str) -> Option<String> {
    let out = git_command(path)
        .args(["rev-parse", "--verify", "--quiet", "refs/stash"])
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run `git` in `path` and return its status, or None if it isn't a git working tree.
fn git_status_for(path: &str, repository_id: i64) -> Option<RepoGitStatus> {
    let branch_out = git_command(path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !branch_out.status.success() {
        return None; // not a git repository
    }
    let branch = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();
    if branch.is_empty() {
        return None;
    }

    let dirty = git_is_dirty(path);

    // `--left-right --count @{upstream}...HEAD` prints "<behind>\t<ahead>"; missing upstream -> 0/0.
    let (ahead, behind) = git_command(path)
        .args([
            "rev-list",
            "--left-right",
            "--count",
            "@{upstream}...HEAD",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            let mut parts = s.split_whitespace();
            let behind = parts.next()?.parse::<i64>().ok()?;
            let ahead = parts.next()?.parse::<i64>().ok()?;
            Some((ahead, behind))
        })
        .unwrap_or((0, 0));

    Some(RepoGitStatus {
        repository_id,
        branch,
        dirty,
        ahead,
        behind,
    })
}

/// Switches `path` to `branch`. If dirty and `stash` is true, stashes first (never silently -
/// callers must pass `stash: true` explicitly). Returns the freshly computed git status.
fn git_switch_branch_impl(
    path: &str,
    branch: &str,
    stash: bool,
    stash_message: &str,
    stash_untracked: bool,
) -> Result<RepoGitStatus, AppError> {
    let dirty = git_is_dirty(path);
    // Whether `git stash push` actually created a stash - NOT the same as `dirty`. A tree that's
    // dirty only with untracked files (`??` in `git status --porcelain`) still leaves `stash push`
    // (without `-u`) a no-op ("No local changes to save", exit 0), so `dirty` alone would
    // incorrectly claim a stash happened. We detect a real stash by comparing the `refs/stash`
    // ref before and after the push - it only changes when a stash was actually created.
    let mut stash_created = false;
    if dirty {
        if !stash {
            return Err(AppError::Launch(
                "repository has uncommitted changes - stash or commit them before switching branches".into(),
            ));
        }
        let before = stash_ref(path);
        let mut stash_args: Vec<&str> = vec!["stash", "push"];
        if stash_untracked {
            stash_args.push("-u");
        }
        if !stash_message.is_empty() {
            stash_args.push("-m");
            stash_args.push(stash_message);
        }
        run_git(path, &stash_args)?;
        stash_created = stash_ref(path) != before;
    }

    // `--` guards against `branch` being smuggled in as a flag (e.g. a leading `-`); the IPC
    // command accepts any string, so this is defense-in-depth even though today's only caller
    // (the branch picker) always passes a real branch name from `git_list_branches`.
    if let Err(e) = run_git(path, &["switch", "--", branch]) {
        return Err(if stash_created {
            AppError::Launch(format!(
                "changes were stashed successfully, but switching to '{branch}' failed: {e}. Run `git stash list` to find your stashed changes."
            ))
        } else {
            e
        });
    }

    // repository_id isn't known inside this pure-path helper; the command wrapper below fills
    // it in via a second git_status_for call keyed by the real repository_id.
    git_status_for(path, 0)
        .ok_or_else(|| AppError::Launch("branch switched, but failed to read updated status".into()))
}

#[tauri::command]
pub async fn git_switch_branch(
    state: State<'_, AppState>,
    repository_id: i64,
    branch: String,
    stash: bool,
    stash_message: String,
    stash_untracked: bool,
) -> Result<RepoGitStatus, AppError> {
    let path = git_repo_path(&state, repository_id).await?;
    let result = git_switch_branch_impl(&path, &branch, stash, &stash_message, stash_untracked);
    // Invalidate unconditionally, even on error: a stash may have already mutated the working
    // tree before the switch itself failed, which would otherwise leave a stale cached status
    // (e.g. still `dirty: true`) that contradicts an error message telling the user their
    // changes were safely stashed. Invalidating when nothing changed is harmless - just a cache
    // miss on the next read.
    state.git_status_cache.invalidate(repository_id);
    let mut status = result?;
    status.repository_id = repository_id;
    Ok(status)
}

/// Fetch from the repository's remote (R5). Invalidates the cached status for this repo -
/// any git write command must do the same (spec 2026-07-31).
#[tauri::command]
pub async fn git_fetch(state: State<'_, AppState>, repository_id: i64) -> Result<String, AppError> {
    let repo = git_repo_path(&state, repository_id).await?;
    let result = run_git(&repo, &["fetch"]);
    state.git_status_cache.invalidate(repository_id);
    result
}

/// Fast-forward pull the repository (R5). `--ff-only` avoids merge prompts/conflicts hanging.
/// Invalidates the cached status for this repo - any git write command must do the same.
#[tauri::command]
pub async fn git_pull(state: State<'_, AppState>, repository_id: i64) -> Result<String, AppError> {
    let repo = git_repo_path(&state, repository_id).await?;
    let result = run_git(&repo, &["pull", "--ff-only"]);
    state.git_status_cache.invalidate(repository_id);
    result
}

async fn git_repo_path(state: &AppState, repository_id: i64) -> Result<String, AppError> {
    persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .map(|r| r.path)
        .ok_or_else(|| AppError::Launch(format!("repository {repository_id} not found")))
}

fn run_git(path: &str, args: &[&str]) -> Result<String, AppError> {
    let out = git_command(path)
        .args(args)
        .output()
        .map_err(|e| AppError::Launch(format!("git failed to run: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(format!("{stdout}{stderr}").trim().to_string())
    } else {
        Err(AppError::Launch(format!("git: {}", stderr.trim())))
    }
}

// ── Settings (F14) ──────────────────────────────────────────────────────────

/// Application settings (F14). Stored as a single JSON blob under the `app_settings` key.
/// `#[serde(default)]` keeps old stored blobs forward-compatible as fields are added.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: String,
    pub launch_delay_ms: i64,
    pub auto_detect: bool,
    pub restore_last_project: bool,
    pub restore_last_selection: bool,
    pub auto_restart: bool,
    pub log_retention: i64,
    pub notifications_enabled: bool,
    pub terminal_shell: String,
    pub visible_actions: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            launch_delay_ms: 1000,
            auto_detect: true,
            restore_last_project: false,
            restore_last_selection: true,
            auto_restart: false,
            log_retention: 1000,
            notifications_enabled: false,
            terminal_shell: "cmd".into(),
            visible_actions: vec![
                "logs".into(),
                "openFolder".into(),
                "openTerminal".into(),
                "edit".into(),
                "remove".into(),
            ],
        }
    }
}

/// Maps a `Settings.terminal_shell` value to the program name used to open an interactive
/// shell window. Unknown/empty values fall back to `"cmd"` (the default), never panicking.
fn shell_program(terminal_shell: &str) -> &'static str {
    if terminal_shell == "powershell" {
        "powershell"
    } else {
        "cmd"
    }
}

/// Builds the (program, args) pair used to run `command_line` in the chosen shell on Windows.
/// `"cmd"` (default) preserves the exact prior behavior; `"powershell"` runs the same command
/// line through `powershell -Command` instead.
#[cfg(windows)]
fn windows_launch_program_args(terminal_shell: &str, command_line: &str) -> (String, Vec<String>) {
    if terminal_shell == "powershell" {
        (
            "powershell".to_string(),
            vec![
                "-NoProfile".to_string(),
                // Windows' default execution policy blocks running the .ps1 shims npm/pnpm/
                // yarn/bun install (npm.ps1 etc.) - without this, every launch fails with
                // "cannot be loaded because running scripts is disabled on this system".
                // Scoped to this one process, not a system-wide policy change.
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                command_line.to_string(),
            ],
        )
    } else {
        ("cmd".to_string(), vec!["/C".to_string(), command_line.to_string()])
    }
}

/// Whether `wt.exe` (Windows Terminal) is resolvable on `PATH`. Checked at call time - no
/// caching - so installing/uninstalling Windows Terminal takes effect on the very next "Open
/// Terminal" click without needing an app restart. Uses `where` (via a hidden `cmd /C` wrapper,
/// matching this file's existing shim-hiding idiom) rather than invoking `wt` itself, since `wt`
/// has no side-effect-free "just check if you exist" flag.
#[cfg(windows)]
fn windows_terminal_available() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd")
        .args(["/C", "where", "wt"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Builds the args for opening a new Windows Terminal tab at `repo_path` running `shell`.
/// `-w 0` targets the most-recently-used `wt` window (creating one if none exists) - this is
/// what gives "one window, many tabs" behavior for free, with no window-tracking of our own.
/// Windows Terminal itself is only ever the UI shell for this blank interactive shell; the
/// process it hosts is never tracked, has no PID recorded, and is not part of any Job Object -
/// unlike a repo's dev-server process (see `process_manager.rs`), it is intentionally outside
/// the app's managed process tree.
fn wt_launch_args(shell: &str, repo_path: &str) -> Vec<String> {
    let mut args = vec![
        "-w".to_string(),
        "0".to_string(),
        "nt".to_string(),
        "-d".to_string(),
        repo_path.to_string(),
        shell.to_string(),
    ];
    if shell == "powershell" {
        // Same execution-policy bypass as windows_launch_program_args - otherwise the first
        // npm/pnpm/yarn/bun command typed in this tab fails to load its .ps1 shim.
        args.extend(["-ExecutionPolicy".to_string(), "Bypass".to_string()]);
    }
    args
}

const SETTINGS_KEY: &str = "app_settings";

/// Loads `Settings` from the `app_settings` blob, falling back to `Settings::default()` if
/// missing or unparseable - the one place every settings read goes through.
async fn load_settings(pool: &sqlx::SqlitePool) -> Result<Settings, AppError> {
    match persistence::get_setting(pool, SETTINGS_KEY)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
    {
        Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
        None => Ok(Settings::default()),
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, AppError> {
    load_settings(&state.pool).await
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, AppError> {
    let json = serde_json::to_string(&settings).map_err(|e| AppError::Persist(e.to_string()))?;
    persistence::set_setting(&state.pool, SETTINGS_KEY, &json)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    Ok(settings)
}

// ── Launch profiles (F6) ────────────────────────────────────────────────────

/// A launch profile: a named, ordered selection of repositories + a launch delay.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub launch_delay_ms: i64,
    pub repository_ids: Vec<i64>,
}

async fn build_profile(
    pool: &sqlx::SqlitePool,
    row: persistence::ProfileRow,
) -> Result<Profile, AppError> {
    let repository_ids = persistence::list_profile_members(pool, row.id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    Ok(Profile {
        id: row.id,
        name: row.name,
        launch_delay_ms: row.launch_delay_ms,
        repository_ids,
    })
}

#[tauri::command]
pub async fn list_profiles(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<Profile>, AppError> {
    let rows = persistence::list_profiles(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(build_profile(&state.pool, row).await?);
    }
    Ok(out)
}

#[tauri::command]
pub async fn create_profile(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
    launch_delay_ms: i64,
    repository_ids: Vec<i64>,
) -> Result<Profile, AppError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Persist("profile name is required".into()));
    }
    let id = persistence::create_profile(&state.pool, project_id, &name, launch_delay_ms)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    persistence::set_profile_members(&state.pool, id, &repository_ids)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let row = persistence::get_profile(&state.pool, id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist("profile not found after create".into()))?;
    build_profile(&state.pool, row).await
}

#[tauri::command]
pub async fn update_profile(
    state: State<'_, AppState>,
    profile_id: i64,
    name: String,
    launch_delay_ms: i64,
    repository_ids: Vec<i64>,
) -> Result<Profile, AppError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Persist("profile name is required".into()));
    }
    persistence::update_profile_meta(&state.pool, profile_id, &name, launch_delay_ms)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    persistence::set_profile_members(&state.pool, profile_id, &repository_ids)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let row = persistence::get_profile(&state.pool, profile_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist("profile not found".into()))?;
    build_profile(&state.pool, row).await
}

#[tauri::command]
pub async fn delete_profile(
    state: State<'_, AppState>,
    profile_id: i64,
) -> Result<(), AppError> {
    persistence::delete_profile(&state.pool, profile_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Apply a profile: enable its members, disable the rest, record it as the last-used profile.
/// Returns the full updated repository list.
#[tauri::command]
pub async fn apply_profile(
    state: State<'_, AppState>,
    profile_id: i64,
) -> Result<Vec<persistence::Repository>, AppError> {
    let row = persistence::get_profile(&state.pool, profile_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist("profile not found".into()))?;
    let members = persistence::list_profile_members(&state.pool, profile_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    persistence::apply_profile(&state.pool, row.project_id, profile_id, &members)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    persistence::list_repositories(&state.pool, row.project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// On-disk shape of an exported profile. Members are referenced by service *name*, not
/// database id - ids are meaningless on a different machine/database, but names are stable
/// across a re-scan of the same project on another machine (or by a teammate).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedProfile {
    name: String,
    launch_delay_ms: i64,
    /// Service names, in launch order.
    members: Vec<String>,
}

/// Writes a profile to `file_path` as JSON, for sharing across machines/teammates (feedback:
/// export/import launch profiles). A plain `&SqlitePool` function (not `State<AppState>`) since
/// it never touches the process manager - this makes it directly unit-testable, unlike commands
/// that need a real `AppHandle`.
async fn export_profile_impl(
    pool: &sqlx::SqlitePool,
    profile_id: i64,
    file_path: &str,
) -> Result<(), AppError> {
    let row = persistence::get_profile(pool, profile_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist("profile not found".into()))?;
    let member_ids = persistence::list_profile_members(pool, profile_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;

    let mut members = Vec::with_capacity(member_ids.len());
    for id in member_ids {
        if let Some(repo) = persistence::get_repository(pool, id)
            .await
            .map_err(|e| AppError::Persist(e.to_string()))?
        {
            members.push(repo.name);
        }
    }

    let exported = ExportedProfile {
        name: row.name,
        launch_delay_ms: row.launch_delay_ms,
        members,
    };
    let json = serde_json::to_string_pretty(&exported)
        .map_err(|e| AppError::Persist(format!("failed to serialize profile: {e}")))?;
    std::fs::write(file_path, json)
        .map_err(|e| AppError::Persist(format!("failed to write {file_path}: {e}")))?;
    Ok(())
}

/// Writes a profile to `file_path` as JSON, for sharing across machines/teammates (feedback:
/// export/import launch profiles).
#[tauri::command]
pub async fn export_profile(
    state: State<'_, AppState>,
    profile_id: i64,
    file_path: String,
) -> Result<(), AppError> {
    export_profile_impl(&state.pool, profile_id, &file_path).await
}

/// Result of `import_profile`: the newly created profile, plus any member names from the file
/// that don't match a service currently discovered in this project (so the caller can surface
/// "imported, but X wasn't found here" instead of silently dropping them).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProfileResult {
    pub profile: Profile,
    pub skipped_members: Vec<String>,
}

/// Reads a profile from `file_path` (written by `export_profile`) and creates it in
/// `project_id`, matching member names against this project's currently-discovered services.
/// A name that already exists in this project gets " (imported)" appended (then " (imported
/// 2)", etc. if that's also taken) rather than rejecting the import or overwriting silently.
/// Rejected outright, before any mutation, if the file has at least one member and NONE of them
/// match anything here - almost certainly the wrong project, not a same-project rename/removal.
/// A plain `&SqlitePool` function for the same testability reason as `export_profile_impl`.
async fn import_profile_impl(
    pool: &sqlx::SqlitePool,
    project_id: i64,
    file_path: &str,
) -> Result<ImportProfileResult, AppError> {
    let contents = std::fs::read_to_string(file_path)
        .map_err(|e| AppError::Persist(format!("failed to read {file_path}: {e}")))?;
    let imported: ExportedProfile = serde_json::from_str(&contents)
        .map_err(|e| AppError::Persist(format!("not a valid profile file: {e}")))?;

    let repos = persistence::list_repositories(pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let mut matched_ids = Vec::new();
    let mut skipped_members = Vec::new();
    for name in &imported.members {
        match repos.iter().find(|r| &r.name == name) {
            Some(repo) => matched_ids.push(repo.id),
            None => skipped_members.push(name.clone()),
        }
    }

    // None of the file's services exist here at all - this is almost certainly the wrong
    // project (or an unrelated one), not a same-project rename/removal. Reject outright rather
    // than silently creating a useless, empty profile with just a warning.
    if !imported.members.is_empty() && matched_ids.is_empty() {
        return Err(AppError::Persist(format!(
            "none of this profile's {} service(s) were found in this project - it looks like it's from a different project",
            imported.members.len()
        )));
    }

    let existing_names: std::collections::HashSet<String> =
        persistence::list_profiles(pool, project_id)
            .await
            .map_err(|e| AppError::Persist(e.to_string()))?
            .into_iter()
            .map(|p| p.name)
            .collect();
    let mut final_name = imported.name.clone();
    if existing_names.contains(&final_name) {
        final_name = format!("{} (imported)", imported.name);
        let mut n = 2;
        while existing_names.contains(&final_name) {
            final_name = format!("{} (imported {n})", imported.name);
            n += 1;
        }
    }

    let id = persistence::create_profile(pool, project_id, &final_name, imported.launch_delay_ms)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    persistence::set_profile_members(pool, id, &matched_ids)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    let row = persistence::get_profile(pool, id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist("profile not found after import".into()))?;
    let profile = build_profile(pool, row).await?;

    Ok(ImportProfileResult {
        profile,
        skipped_members,
    })
}

/// Reads a profile from `file_path` (written by `export_profile`) and creates it in
/// `project_id`, matching member names against this project's currently-discovered services.
#[tauri::command]
pub async fn import_profile(
    state: State<'_, AppState>,
    project_id: i64,
    file_path: String,
) -> Result<ImportProfileResult, AppError> {
    import_profile_impl(&state.pool, project_id, &file_path).await
}

/// Update a repository's command configuration (F5), returning the updated row.
/// Empty strings clear the corresponding override (revert to the detected default).
#[tauri::command]
pub async fn update_repository_config(
    state: State<'_, AppState>,
    repository_id: i64,
    package_manager: String,
    command: String,
    args: String,
    env_file: String,
) -> Result<persistence::Repository, AppError> {
    const VALID_PM: [&str; 4] = ["npm", "pnpm", "yarn", "bun"];
    if !VALID_PM.contains(&package_manager.as_str()) {
        return Err(AppError::Persist(format!(
            "invalid package manager: {package_manager}"
        )));
    }
    let opt = |s: String| {
        let t = s.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
    let (command, args, env_file) = (opt(command), opt(args), opt(env_file));
    persistence::update_repository_config(
        &state.pool,
        repository_id,
        &package_manager,
        command.as_deref(),
        args.as_deref(),
        env_file.as_deref(),
    )
    .await
    .map_err(|e| AppError::Persist(e.to_string()))
}

/// Remove a repository from a project (F16, soft delete). Stops it first if running (FR-32),
/// preserves history, and returns the project's remaining active repositories.
#[tauri::command]
pub async fn remove_repository(
    state: State<'_, AppState>,
    repository_id: i64,
) -> Result<Vec<persistence::Repository>, AppError> {
    let repo = persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Persist(format!("repository {repository_id} not found")))?;

    // Stop it first if it's running (ignore "not running").
    let _ = state.process_manager.stop(repository_id);

    persistence::remove_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    persistence::list_repositories(&state.pool, repo.project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Star/unstar a repository (R7), returning the updated row.
#[tauri::command]
pub async fn set_repository_favorite(
    state: State<'_, AppState>,
    repository_id: i64,
    favorite: bool,
) -> Result<persistence::Repository, AppError> {
    persistence::set_repository_favorite(&state.pool, repository_id, favorite)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Toggle whether a repository launches in a visible interactive console window instead of
/// piped background logs, returning the updated row.
#[tauri::command]
pub async fn set_repository_visible_console(
    state: State<'_, AppState>,
    repository_id: i64,
    visible_console: bool,
) -> Result<persistence::Repository, AppError> {
    persistence::set_repository_visible_console(&state.pool, repository_id, visible_console)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

/// Enable/disable a repository (F4), returning the updated row.
#[tauri::command]
pub async fn set_repository_enabled(
    state: State<'_, AppState>,
    repository_id: i64,
    enabled: bool,
) -> Result<persistence::Repository, AppError> {
    persistence::set_repository_enabled(&state.pool, repository_id, enabled)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))
}

// ── Process control (F9) ───────────────────────────────────────────────────

/// Start a repository's dev process (F9).
#[tauri::command]
pub async fn start_repo(
    state: State<'_, AppState>,
    repository_id: i64,
) -> Result<StatusUpdate, AppError> {
    let spec = launch_spec_for(&state, repository_id).await?;
    state
        .process_manager
        .start(repository_id, spec, None)
        .await
        .map_err(AppError::Launch)
}

/// Stop a repository's process tree (F9, forced Job Object kill).
#[tauri::command]
pub async fn stop_repo(state: State<'_, AppState>, repository_id: i64) -> Result<(), AppError> {
    state
        .process_manager
        .stop(repository_id)
        .map_err(AppError::Launch)
}

/// Restart a repository (Stop then Start, incrementing restart count) (F9).
#[tauri::command]
pub async fn restart_repo(
    state: State<'_, AppState>,
    repository_id: i64,
) -> Result<StatusUpdate, AppError> {
    state
        .process_manager
        .restart(repository_id)
        .await
        .map_err(AppError::Launch)
}

/// Resolve a repository's launch command. On Windows dev-server launchers (`npm`/`pnpm`/`yarn`/
/// `bun`) are `.cmd` shims, so we run through `cmd /C <…>` - this is exactly why the Job Object
/// tree-kill (ADR-0003) is required.
async fn launch_spec_for(
    state: &AppState,
    repository_id: i64,
) -> Result<LaunchSpec, AppError> {
    let repo = persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Launch(format!("repository {repository_id} not found")))?;

    // Fail with a clear message rather than letting the spawn itself hit the OS with a
    // nonexistent cwd (which surfaces as an opaque "The directory name is invalid" error) - a
    // repo's folder can vanish (moved/deleted outside the app) without the launcher noticing
    // until the user tries to act on it.
    if !std::path::Path::new(&repo.path).is_dir() {
        return Err(AppError::Launch(format!(
            "repository '{}' folder no longer exists: {}",
            repo.name, repo.path
        )));
    }

    // Build the command tokens: a user override runs verbatim; otherwise derive from the
    // package manager + detected script (design.md §1.4).
    let mut tokens: Vec<String> = Vec::new();
    if let Some(command) = repo.command.as_deref().filter(|c| !c.is_empty()) {
        tokens.push(command.to_string());
    } else if let Some(script) = repo.detected_script.as_deref() {
        match repo.package_manager.as_str() {
            "yarn" => tokens.push(format!("yarn {script}")),
            pm => tokens.push(format!("{pm} run {script}")),
        }
    } else {
        return Err(AppError::Launch(format!(
            "repository '{}' has no launch command (set one or add a start script)",
            repo.name
        )));
    }
    if let Some(args) = repo.args.as_deref().filter(|a| !a.is_empty()) {
        tokens.push(args.to_string());
    }
    let command_line = tokens.join(" ");

    // Load env vars from the repo's env file (F5), resolved relative to the repo dir. Best-effort:
    // a missing/unreadable env file is skipped (the process just runs without those vars).
    let env = repo
        .env_file
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(|f| load_env_file(&repo.path, f))
        .unwrap_or_default();

    let visible = repo.visible_console != 0;

    #[cfg(windows)]
    {
        let settings = load_settings(&state.pool).await?;
        let (program, args) = windows_launch_program_args(&settings.terminal_shell, &command_line);
        Ok(LaunchSpec {
            program,
            args,
            cwd: repo.path,
            env,
            visible,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(LaunchSpec {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), command_line],
            cwd: repo.path,
            env,
            visible,
        })
    }
}

/// Parse a simple `.env` file (KEY=VALUE per line; `#` comments and blanks ignored; surrounding
/// double quotes on the value stripped). `env_file` may be absolute or relative to `repo_path`.
/// ponytail: naive parser - no multiline/escape handling; upgrade to a dotenv crate if needed.
fn load_env_file(repo_path: &str, env_file: &str) -> Vec<(String, String)> {
    let path = {
        let p = std::path::Path::new(env_file);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::path::Path::new(repo_path).join(env_file)
        }
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_string(),
                v.trim().trim_matches('"').to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
mod git_tests {
    use super::*;
    use std::process::Command;

    /// Creates a temp dir with one commit, cleaned up by the caller via `remove_dir_all`.
    fn init_temp_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mrl-commands-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp repo dir");
        Command::new("git").args(["init", "-q"]).current_dir(&dir).output().unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir)
            .output()
            .unwrap();
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        Command::new("git").args(["add", "."]).current_dir(&dir).output().unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "init"])
            .current_dir(&dir)
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn run_git_reports_clean_status_via_git_command_helper() {
        let dir = init_temp_repo();
        let out = run_git(dir.to_str().unwrap(), &["status", "--porcelain"]).unwrap();
        assert_eq!(out, "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_status_for_returns_branch_and_clean_flag() {
        let dir = init_temp_repo();
        let status = git_status_for(dir.to_str().unwrap(), 1).expect("should detect git repo");
        assert!(!status.branch.is_empty());
        assert!(!status.dirty);
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_status_for_returns_none_for_non_git_directory() {
        let dir = std::env::temp_dir().join(format!("mrl-commands-notgit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(git_status_for(dir.to_str().unwrap(), 1).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_status_project_serves_second_call_from_cache() {
        let dir = init_temp_repo();
        let cache = std::sync::Arc::new(crate::git_status_cache::GitStatusCache::new());
        let canonical_path = std::path::PathBuf::from(dir.to_str().unwrap());

        // First call: cache miss, computed directly (bypassing the Tauri command wrapper, which
        // needs a full AppState/sqlite pool - this exercises the same cache + git_status_for path
        // git_status_project uses).
        assert!(cache.get(1, &canonical_path, crate::git_status_cache::GIT_STATUS_CACHE_TTL).is_none());
        let status = git_status_for(dir.to_str().unwrap(), 1).expect("git repo");
        cache.insert(1, canonical_path.clone(), status.clone());

        // Second call within TTL: cache hit, no git subprocess needed.
        let cached = cache.get(1, &canonical_path, crate::git_status_cache::GIT_STATUS_CACHE_TTL);
        assert_eq!(cached.unwrap().branch, status.branch);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;
    use std::process::Command;

    fn init_temp_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mrl-branch-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp repo dir");
        Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(&dir).output().unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir)
            .output()
            .unwrap();
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        Command::new("git").args(["add", "."]).current_dir(&dir).output().unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "init"])
            .current_dir(&dir)
            .output()
            .unwrap();
        dir
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn git_is_dirty_reports_clean_and_dirty() {
        let dir = init_temp_repo();
        assert!(!git_is_dirty(dir.to_str().unwrap()));
        std::fs::write(dir.join("a.txt"), "changed").unwrap();
        assert!(git_is_dirty(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_list_branches_lists_local_branches_current_first() {
        let dir = init_temp_repo();
        git(&dir, &["branch", "feature-a"]);
        git(&dir, &["branch", "aardvark"]);

        let branches = git_list_branches_impl(dir.to_str().unwrap()).unwrap();
        let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["main", "aardvark", "feature-a"]);
        assert!(branches[0].is_current);
        assert!(!branches[1].is_current);
        assert!(!branches[2].is_current);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_list_branches_orders_common_branch_names_before_others() {
        let dir = init_temp_repo();
        git(&dir, &["checkout", "-q", "-b", "zeta"]);
        git(&dir, &["branch", "develop"]);
        git(&dir, &["branch", "master"]);

        let branches = git_list_branches_impl(dir.to_str().unwrap()).unwrap();
        let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
        // current ("zeta") first; the other three branches (main, develop, master) are ALL
        // common names, so they sort by COMMON_BRANCH_ORDER's fixed order (main, master,
        // develop), not alphabetically - there's nothing left in the "rest" bucket here.
        assert_eq!(names, vec!["zeta", "main", "master", "develop"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_list_branches_excludes_remote_head_and_dedupes_by_name() {
        // Bare repo acting as `origin`, plus a clone with a second branch pushed only there.
        let bare_dir = std::env::temp_dir().join(format!("mrl-branch-bare-{}", std::process::id()));
        std::fs::create_dir_all(&bare_dir).unwrap();
        git(&bare_dir, &["init", "-q", "--bare", "-b", "main"]);

        let clone_dir = init_temp_repo();
        git(&clone_dir, &["remote", "add", "origin", bare_dir.to_str().unwrap()]);
        git(&clone_dir, &["push", "-q", "origin", "main"]);
        git(&clone_dir, &["checkout", "-q", "-b", "feature-b"]);
        git(&clone_dir, &["push", "-q", "-u", "origin", "feature-b"]);
        git(&clone_dir, &["checkout", "-q", "main"]);
        git(&clone_dir, &["branch", "-D", "feature-b"]); // now only exists on origin
        git(&clone_dir, &["fetch", "-q"]);

        let branches = git_list_branches_impl(clone_dir.to_str().unwrap()).unwrap();
        let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"feature-b"), "expected remote-only branch to appear: {names:?}");
        assert_eq!(names.iter().filter(|n| **n == "main").count(), 1, "main must not be duplicated");
        assert!(!names.iter().any(|n| n.contains("HEAD")), "origin/HEAD must be excluded");

        let _ = std::fs::remove_dir_all(&bare_dir);
        let _ = std::fs::remove_dir_all(&clone_dir);
    }

    #[tokio::test]
    async fn git_list_branches_excludes_detached_head_pseudo_entry() {
        let dir = init_temp_repo();
        git(&dir, &["branch", "feature-detached"]);

        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&dir)
            .output()
            .unwrap();
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        git(&dir, &["checkout", "-q", "--detach", &sha]);

        let branches = git_list_branches_impl(dir.to_str().unwrap()).unwrap();
        let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
        assert!(
            !names.iter().any(|n| *n == "HEAD" || n.contains("HEAD")),
            "detached HEAD pseudo-entry must be excluded: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with('(')),
            "pseudo-entry like '(HEAD detached at ...)' must be excluded: {names:?}"
        );
        assert!(names.contains(&"main"), "pre-existing local branch must remain: {names:?}");
        assert!(names.contains(&"feature-detached"), "pre-existing local branch must remain: {names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_switch_branch_succeeds_on_clean_repo() {
        let dir = init_temp_repo();
        git(&dir, &["branch", "feature-c"]);

        let status = git_switch_branch_impl(dir.to_str().unwrap(), "feature-c", false, "", false).unwrap();
        assert_eq!(status.branch, "feature-c");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_switch_branch_rejects_dirty_switch_without_stash() {
        let dir = init_temp_repo();
        git(&dir, &["branch", "feature-d"]);
        std::fs::write(dir.join("a.txt"), "dirty").unwrap();

        let result = git_switch_branch_impl(dir.to_str().unwrap(), "feature-d", false, "", false);
        assert!(result.is_err());
        // Branch must be unchanged.
        let branch_out = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&branch_out.stdout).trim(), "main");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_switch_branch_stashes_with_custom_message_then_switches() {
        let dir = init_temp_repo();
        git(&dir, &["branch", "feature-e"]);
        std::fs::write(dir.join("a.txt"), "dirty again").unwrap();

        let status = git_switch_branch_impl(dir.to_str().unwrap(), "feature-e", true, "my wip", false).unwrap();
        assert_eq!(status.branch, "feature-e");
        assert!(!status.dirty, "stash should have cleared the working tree");

        let stash_out = Command::new("git").args(["stash", "list"]).current_dir(&dir).output().unwrap();
        let stash_list = String::from_utf8_lossy(&stash_out.stdout);
        assert!(stash_list.contains("my wip"), "expected custom stash message in: {stash_list}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_switch_branch_to_remote_only_branch_creates_tracking_branch() {
        let bare_dir = std::env::temp_dir().join(format!("mrl-branch-bare2-{}", std::process::id()));
        std::fs::create_dir_all(&bare_dir).unwrap();
        git(&bare_dir, &["init", "-q", "--bare", "-b", "main"]);

        let clone_dir = init_temp_repo();
        git(&clone_dir, &["remote", "add", "origin", bare_dir.to_str().unwrap()]);
        git(&clone_dir, &["push", "-q", "origin", "main"]);
        git(&clone_dir, &["checkout", "-q", "-b", "feature-f"]);
        git(&clone_dir, &["push", "-q", "-u", "origin", "feature-f"]);
        git(&clone_dir, &["checkout", "-q", "main"]);
        git(&clone_dir, &["branch", "-D", "feature-f"]);
        git(&clone_dir, &["fetch", "-q"]);

        let status = git_switch_branch_impl(clone_dir.to_str().unwrap(), "feature-f", false, "", false).unwrap();
        assert_eq!(status.branch, "feature-f");

        let upstream_out = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "feature-f@{upstream}"])
            .current_dir(&clone_dir)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&upstream_out.stdout).trim(), "origin/feature-f");

        let _ = std::fs::remove_dir_all(&bare_dir);
        let _ = std::fs::remove_dir_all(&clone_dir);
    }

    #[tokio::test]
    async fn git_switch_branch_untracked_only_does_not_create_real_stash() {
        // Untracked-only dirty tree, stash requested but stash_untracked=false: `git stash push`
        // (no `-u`) has nothing to stash, so no real stash should be created, and the switch
        // (to a real, existing branch) should still succeed since untracked files don't block it.
        let dir = init_temp_repo();
        git(&dir, &["branch", "feature-untracked"]);
        std::fs::write(dir.join("untracked.txt"), "new file").unwrap();

        let status =
            git_switch_branch_impl(dir.to_str().unwrap(), "feature-untracked", true, "", false).unwrap();
        assert_eq!(status.branch, "feature-untracked");

        let stash_out = Command::new("git").args(["stash", "list"]).current_dir(&dir).output().unwrap();
        assert!(
            String::from_utf8_lossy(&stash_out.stdout).trim().is_empty(),
            "no real stash should have been created for untracked-only changes without -u"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_switch_branch_untracked_only_switch_failure_does_not_claim_stash_success() {
        // Same untracked-only setup as above, but the switch itself fails (nonexistent target
        // branch). This is the scenario that actually exposes the did_stash/was-dirty conflation:
        // pre-fix, `did_stash` was set from the pre-check dirty flag (true, because of the
        // untracked file) even though `git stash push` (no -u) created no stash at all, so the
        // error would falsely claim "changes were stashed successfully".
        let dir = init_temp_repo();
        std::fs::write(dir.join("untracked2.txt"), "new file").unwrap();

        let result =
            git_switch_branch_impl(dir.to_str().unwrap(), "this-branch-does-not-exist", true, "", false);
        let err = result.expect_err("switch to a nonexistent branch must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains("stashed successfully"),
            "must not claim a stash succeeded when nothing was actually stashed: {msg}"
        );

        let stash_out = Command::new("git").args(["stash", "list"]).current_dir(&dir).output().unwrap();
        assert!(
            String::from_utf8_lossy(&stash_out.stdout).trim().is_empty(),
            "no stash should exist since nothing was actually stashed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_switch_branch_stash_succeeds_but_switch_to_nonexistent_branch_fails() {
        // Case "d": a real stash IS created (tracked-file change), but the subsequent switch
        // fails. The error must use the "stashed successfully" wording, and `git stash list`
        // must show exactly one real entry, proving the claim is accurate.
        let dir = init_temp_repo();
        std::fs::write(dir.join("a.txt"), "dirty tracked change").unwrap();

        let result =
            git_switch_branch_impl(dir.to_str().unwrap(), "this-branch-does-not-exist", true, "", false);
        let err = result.expect_err("switch to a nonexistent branch must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("stashed successfully"),
            "expected stash-succeeded wording, got: {msg}"
        );

        let stash_out = Command::new("git").args(["stash", "list"]).current_dir(&dir).output().unwrap();
        let stash_list = String::from_utf8_lossy(&stash_out.stdout);
        assert_eq!(
            stash_list.lines().filter(|l| !l.is_empty()).count(),
            1,
            "expected exactly one real stash entry: {stash_list}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn shell_program_maps_powershell_and_defaults_to_cmd() {
        assert_eq!(shell_program("powershell"), "powershell");
        assert_eq!(shell_program("cmd"), "cmd");
        assert_eq!(shell_program(""), "cmd");
        assert_eq!(shell_program("bash"), "cmd"); // unknown value falls back safely
    }

    #[cfg(windows)]
    #[test]
    fn windows_launch_program_args_switches_on_terminal_shell() {
        let (program, args) = windows_launch_program_args("cmd", "npm run dev");
        assert_eq!(program, "cmd");
        assert_eq!(args, vec!["/C".to_string(), "npm run dev".to_string()]);

        let (program, args) = windows_launch_program_args("powershell", "npm run dev");
        assert_eq!(program, "powershell");
        assert_eq!(
            args,
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                "npm run dev".to_string()
            ]
        );
    }

    #[test]
    fn settings_default_terminal_shell_is_cmd() {
        assert_eq!(Settings::default().terminal_shell, "cmd");
    }

    #[test]
    fn wt_launch_args_targets_most_recently_used_window_and_sets_starting_dir() {
        let args = wt_launch_args("cmd", "C:\\repos\\api");
        assert_eq!(
            args,
            vec![
                "-w".to_string(),
                "0".to_string(),
                "nt".to_string(),
                "-d".to_string(),
                "C:\\repos\\api".to_string(),
                "cmd".to_string(),
            ]
        );
    }

    #[test]
    fn wt_launch_args_adds_execution_policy_bypass_for_powershell() {
        let args = wt_launch_args("powershell", "C:\\repos\\api");
        assert_eq!(
            args,
            vec![
                "-w".to_string(),
                "0".to_string(),
                "nt".to_string(),
                "-d".to_string(),
                "C:\\repos\\api".to_string(),
                "powershell".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
            ]
        );
    }

    #[test]
    fn settings_default_visible_actions_is_curated_set() {
        let defaults = Settings::default();
        assert_eq!(
            defaults.visible_actions,
            vec!["logs", "openFolder", "openTerminal", "edit", "remove"]
        );
    }

    #[test]
    fn old_settings_json_without_visible_actions_falls_back_to_default() {
        // Simulates a stored blob from before this field existed.
        let old_json = r#"{"theme":"dark","launchDelayMs":500,"autoDetect":true,
            "restoreLastProject":true,"restoreLastSelection":true,"autoRestart":false,
            "logRetention":1000,"notificationsEnabled":true,"terminalShell":"cmd"}"#;
        let parsed: Settings = serde_json::from_str(old_json).unwrap();
        assert_eq!(
            parsed.visible_actions,
            vec!["logs", "openFolder", "openTerminal", "edit", "remove"]
        );
        // Fields that WERE present in the old blob are preserved, not overwritten by defaults.
        assert_eq!(parsed.theme, "dark");
        assert_eq!(parsed.launch_delay_ms, 500);
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;

    #[test]
    fn validate_project_name_rejects_empty_and_whitespace() {
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("   ").is_err());
        assert_eq!(validate_project_name("  My Project  ").unwrap(), "My Project");
    }
}

#[cfg(test)]
mod path_management_tests {
    use super::*;

    /// Creates a unique temp directory, cleaned up via `Drop`. Local to this module, matching
    /// the existing per-module temp-dir helper convention already used by `scanner.rs` and this
    /// file's `git_tests`.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mrl-path-mgmt-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_pkg_json(dir: &std::path::Path, subpath: &str) {
        let full = dir.join(subpath);
        std::fs::create_dir_all(&full).expect("create repo dir");
        std::fs::write(full.join("package.json"), r#"{"scripts": {"dev": "node ."}}"#)
            .expect("write package.json");
    }

    fn fake_repo(id: i64, path: &std::path::Path, name: &str) -> persistence::Repository {
        persistence::Repository {
            id,
            project_id: 1,
            name: name.to_string(),
            path: path.to_str().unwrap().to_string(),
            package_manager: "npm".to_string(),
            detected_script: None,
            command: None,
            args: None,
            env_file: None,
            enabled: 1,
            favorite: 0,
            visible_console: 0,
            removed_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn remaps_repo_that_exists_at_the_same_relative_path() {
        let old = TempDir::new();
        let new = TempDir::new();
        write_pkg_json(old.path(), "api");
        write_pkg_json(new.path(), "api");

        let repo = fake_repo(1, &old.path().join("api"), "api");
        let remap = compute_repo_remap(old.path(), new.path(), &[repo]);

        assert_eq!(remap.len(), 1);
        assert_eq!(remap[0].repository_id, 1);
        assert_eq!(remap[0].path, new.path().join("api").display().to_string());
    }

    #[test]
    fn excludes_repo_whose_new_location_has_no_package_json() {
        let old = TempDir::new();
        let new = TempDir::new();
        write_pkg_json(old.path(), "api");
        std::fs::create_dir_all(new.path().join("api")).expect("create dir without package.json");

        let repo = fake_repo(1, &old.path().join("api"), "api");
        let remap = compute_repo_remap(old.path(), new.path(), &[repo]);

        assert!(remap.is_empty());
    }

    #[test]
    fn remaps_nested_relative_paths() {
        let old = TempDir::new();
        let new = TempDir::new();
        write_pkg_json(old.path(), "services/notifications");
        write_pkg_json(new.path(), "services/notifications");

        let repo = fake_repo(1, &old.path().join("services/notifications"), "services/notifications");
        let remap = compute_repo_remap(old.path(), new.path(), &[repo]);

        assert_eq!(remap.len(), 1);
        assert_eq!(remap[0].path, new.path().join("services/notifications").display().to_string());
    }

    #[test]
    fn handles_multiple_repositories_independently() {
        let old = TempDir::new();
        let new = TempDir::new();
        write_pkg_json(old.path(), "api");
        write_pkg_json(old.path(), "web");
        write_pkg_json(new.path(), "api"); // only "api" exists under the new root
        std::fs::create_dir_all(new.path().join("web-renamed")).unwrap(); // "web" isn't here

        let repos = vec![
            fake_repo(1, &old.path().join("api"), "api"),
            fake_repo(2, &old.path().join("web"), "web"),
        ];
        let remap = compute_repo_remap(old.path(), new.path(), &repos);

        assert_eq!(remap.len(), 1, "only api should remap; web has no match under the new root");
        assert_eq!(remap[0].repository_id, 1);
    }

    #[test]
    fn excludes_repo_whose_relative_path_does_not_exist_under_new_root_at_all() {
        let old = TempDir::new();
        let new = TempDir::new();
        write_pkg_json(old.path(), "api");
        // new root has nothing under "api" at all, not even an empty directory.

        let repo = fake_repo(1, &old.path().join("api"), "api");
        let remap = compute_repo_remap(old.path(), new.path(), &[repo]);

        assert!(remap.is_empty());
    }

    /// Create a fresh temp SQLite DB, migrated and ready to use. Mirrors persistence.rs's own
    /// test helper - duplicated locally since that one is private to persistence.rs's test
    /// module, matching this codebase's existing convention of small per-module test helpers.
    async fn setup_test_db() -> (sqlx::SqlitePool, std::path::PathBuf) {
        let unique = format!(
            "mrl_cmd_test_{}_{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let db_path = std::env::temp_dir().join(unique);
        let pool = persistence::init_pool(&db_path).await.expect("init_pool failed");
        persistence::run_migrations(&pool).await.expect("run_migrations failed");
        (pool, db_path)
    }

    fn cleanup_db(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[tokio::test]
    async fn update_project_path_remaps_existing_and_picks_up_new_repos() {
        let (pool, db_path) = setup_test_db().await;
        let old_root = TempDir::new();
        let new_root = TempDir::new();

        write_pkg_json(old_root.path(), "api");
        write_pkg_json(old_root.path(), "worker");
        write_pkg_json(new_root.path(), "api"); // same relative path -> should remap
        write_pkg_json(new_root.path(), "web"); // brand new under the new root -> picked up by scan
        // note: no "worker" dir under new_root -> that repo can't remap and must be left untouched

        let project = persistence::upsert_project(&pool, "Proj", old_root.path().to_str().unwrap())
            .await
            .expect("upsert project failed");
        let old_api_path = old_root.path().join("api").to_str().unwrap().to_string();
        let repo = persistence::upsert_repository(&pool, project.id, "api", &old_api_path, "npm", Some("dev"), true)
            .await
            .expect("upsert repository failed");
        persistence::update_repository_config(&pool, repo.id, "npm", Some("npm run custom"), None, None)
            .await
            .expect("set command override failed");
        let old_worker_path = old_root.path().join("worker").to_str().unwrap().to_string();
        let worker_repo = persistence::upsert_repository(&pool, project.id, "worker", &old_worker_path, "npm", Some("dev"), true)
            .await
            .expect("upsert worker repository failed");

        // Mirrors update_project_path's body, bypassing the Tauri command wrapper (which needs a
        // real AppHandle/AppState) -- same approach this file's existing git_tests already use.
        let existing = persistence::list_repositories(&pool, project.id).await.expect("list failed");
        let remap = compute_repo_remap(old_root.path(), new_root.path(), &existing);
        persistence::apply_project_remap(&pool, project.id, new_root.path().to_str().unwrap(), &remap)
            .await
            .expect("apply_project_remap failed");
        let repositories = discover_and_persist(&pool, project.id, new_root.path())
            .await
            .expect("discover_and_persist failed");

        let api = repositories.iter().find(|r| r.name == "api").expect("api must still be present");
        assert_eq!(api.id, repo.id, "remapped repo must keep its original id");
        assert_eq!(api.path, new_root.path().join("api").display().to_string());
        assert_eq!(api.command.as_deref(), Some("npm run custom"), "override must survive the move");

        let web = repositories.iter().find(|r| r.name == "web");
        assert!(web.is_some(), "brand-new repo under the new root must be picked up by the scan");

        let worker = repositories
            .iter()
            .find(|r| r.name == "worker")
            .expect("worker must still be present (never auto-removed)");
        assert_eq!(worker.id, worker_repo.id, "untouched repo must keep its original id");
        assert_eq!(
            worker.path, old_worker_path,
            "repo that couldn't remap must be left at its old path, unchanged"
        );

        pool.close().await;
        cleanup_db(&db_path);
    }

    #[tokio::test]
    async fn update_repository_path_command_rejects_missing_package_json_and_succeeds_otherwise() {
        let (pool, db_path) = setup_test_db().await;
        let old_dir = TempDir::new();
        let new_dir = TempDir::new();
        write_pkg_json(old_dir.path(), "api");

        let project = persistence::upsert_project(&pool, "Proj", old_dir.path().to_str().unwrap())
            .await
            .expect("upsert project failed");
        let old_api_path = old_dir.path().join("api").to_str().unwrap().to_string();
        let repo = persistence::upsert_repository(&pool, project.id, "api", &old_api_path, "npm", Some("dev"), true)
            .await
            .expect("upsert repository failed");

        // No package.json here -> classify_repo_dir returns None -> the command must reject.
        let empty_target = new_dir.path().join("empty");
        std::fs::create_dir_all(&empty_target).unwrap();
        assert!(
            scanner::classify_repo_dir(&empty_target, None).is_none(),
            "sanity check: this fixture must have no package.json"
        );

        // Valid target with a different leaf name -> name must update to match (mirrors what
        // the update_repository_path command does internally: classify, then persist).
        write_pkg_json(new_dir.path(), "billing");
        let target = new_dir.path().join("billing");
        let discovered = scanner::classify_repo_dir(&target, None).expect("target must classify as a repo");
        let updated = persistence::update_repository_path(
            &pool,
            repo.id,
            &discovered.path,
            &discovered.name,
            discovered.package_manager.as_str(),
            discovered.detected_script.as_deref(),
        )
        .await
        .expect("update_repository_path failed");

        assert_eq!(updated.name, "billing", "name must follow the new folder");
        assert_eq!(updated.path, target.display().to_string());

        pool.close().await;
        cleanup_db(&db_path);
    }

    /// Mirrors `refresh_repositories`' body (bypassing the Tauri command wrapper, which needs a
    /// real AppHandle/AppState - same approach `update_project_path`'s test above already uses)
    /// against a real DB + real filesystem, returning what the command would return.
    async fn run_refresh_repositories(
        pool: &sqlx::SqlitePool,
        project: &persistence::Project,
    ) -> (Vec<persistence::Repository>, Vec<i64>, bool) {
        let root_missing = !std::path::Path::new(&project.root_path).is_dir();
        let mut repositories = persistence::list_repositories(pool, project.id).await.expect("list failed");
        let mut missing_repository_ids = Vec::new();

        if !root_missing {
            for repo in repositories.iter_mut() {
                let dir = std::path::Path::new(&repo.path);
                match scanner::classify_repo_dir(dir, Some(&repo.name)) {
                    Some(discovered) => {
                        if discovered.package_manager.as_str() != repo.package_manager
                            || discovered.detected_script != repo.detected_script
                        {
                            *repo = persistence::refresh_repository_detection(
                                pool,
                                repo.id,
                                discovered.package_manager.as_str(),
                                discovered.detected_script.as_deref(),
                            )
                            .await
                            .expect("refresh_repository_detection failed");
                        }
                    }
                    None => missing_repository_ids.push(repo.id),
                }
            }
        }

        (repositories, missing_repository_ids, root_missing)
    }

    #[tokio::test]
    async fn refresh_repositories_updates_detection_and_ignores_removed_repos() {
        let (pool, db_path) = setup_test_db().await;
        let root = TempDir::new();
        write_pkg_json(root.path(), "api");
        write_pkg_json(root.path(), "gone");

        let project = persistence::upsert_project(&pool, "Proj", root.path().to_str().unwrap())
            .await
            .expect("upsert project failed");
        let api_path = root.path().join("api").to_str().unwrap().to_string();
        let api = persistence::upsert_repository(&pool, project.id, "api", &api_path, "npm", Some("dev"), true)
            .await
            .expect("upsert api failed");

        // "removed" was deliberately removed by the user; its folder no longer exists on disk
        // either, but that must not matter -- it's inactive and must be left out entirely.
        let gone_path = root.path().join("gone").to_str().unwrap().to_string();
        let removed = persistence::upsert_repository(&pool, project.id, "gone", &gone_path, "npm", Some("dev"), true)
            .await
            .expect("upsert removed failed");
        persistence::remove_repository(&pool, removed.id).await.expect("remove_repository failed");
        std::fs::remove_dir_all(root.path().join("gone")).expect("delete gone dir");

        // Simulate a package manager change + a new script appearing since the last scan.
        std::fs::write(root.path().join("api").join("pnpm-lock.yaml"), "").expect("write lockfile");
        std::fs::write(
            root.path().join("api").join("package.json"),
            r#"{"scripts": {"start:dev": "node ."}}"#,
        )
        .expect("rewrite package.json");

        let (repositories, missing, root_missing) = run_refresh_repositories(&pool, &project).await;

        assert!(!root_missing);
        assert!(missing.is_empty(), "no active repo should be reported missing");
        assert_eq!(repositories.len(), 1, "removed repo must not reappear");
        let api_after = repositories.iter().find(|r| r.id == api.id).expect("api must be present");
        assert_eq!(api_after.package_manager, "pnpm", "package manager must be refreshed");
        assert_eq!(api_after.detected_script.as_deref(), Some("start:dev"), "script must be refreshed");
        assert_eq!(api_after.path, api_path, "path must be untouched by a detection refresh");

        pool.close().await;
        cleanup_db(&db_path);
    }

    #[tokio::test]
    async fn refresh_repositories_flags_missing_repo_without_mutating_it() {
        let (pool, db_path) = setup_test_db().await;
        let root = TempDir::new();
        write_pkg_json(root.path(), "api");

        let project = persistence::upsert_project(&pool, "Proj", root.path().to_str().unwrap())
            .await
            .expect("upsert project failed");
        let api_path = root.path().join("api").to_str().unwrap().to_string();
        let api = persistence::upsert_repository(&pool, project.id, "api", &api_path, "npm", Some("dev"), true)
            .await
            .expect("upsert api failed");

        // Folder deleted outside the app -- not a deliberate Remove.
        std::fs::remove_dir_all(root.path().join("api")).expect("delete api dir");

        let (repositories, missing, root_missing) = run_refresh_repositories(&pool, &project).await;

        assert!(!root_missing);
        assert_eq!(missing, vec![api.id]);
        let api_after = repositories.iter().find(|r| r.id == api.id).expect("api row must still be present");
        assert_eq!(api_after.path, api_path, "row must be completely untouched, not deleted or repathed");
        assert_eq!(api_after.package_manager, "npm");

        pool.close().await;
        cleanup_db(&db_path);
    }

    #[tokio::test]
    async fn refresh_repositories_reports_root_missing_and_skips_per_repo_checks() {
        let (pool, db_path) = setup_test_db().await;
        let root = TempDir::new();
        write_pkg_json(root.path(), "api");

        let project = persistence::upsert_project(&pool, "Proj", root.path().to_str().unwrap())
            .await
            .expect("upsert project failed");
        persistence::upsert_repository(
            &pool,
            project.id,
            "api",
            root.path().join("api").to_str().unwrap(),
            "npm",
            Some("dev"),
            true,
        )
        .await
        .expect("upsert api failed");

        // The whole project root is gone, not just one repo.
        std::fs::remove_dir_all(root.path()).expect("delete project root");

        let (repositories, missing, root_missing) = run_refresh_repositories(&pool, &project).await;

        assert!(root_missing);
        assert!(missing.is_empty(), "per-repo checks must be skipped when the root itself is missing");
        assert_eq!(repositories.len(), 1, "repo row must still be returned, just unexamined");

        pool.close().await;
        cleanup_db(&db_path);
    }
}

#[cfg(test)]
mod profile_export_import_tests {
    use super::*;

    async fn setup_test_db() -> (sqlx::SqlitePool, std::path::PathBuf) {
        let unique = format!(
            "mrl_profile_test_{}_{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let db_path = std::env::temp_dir().join(unique);
        let pool = persistence::init_pool(&db_path).await.expect("init_pool failed");
        persistence::run_migrations(&pool).await.expect("run_migrations failed");
        (pool, db_path)
    }

    fn cleanup_db(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    fn temp_json_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "mrl-profile-export-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn export_then_import_round_trips_name_delay_and_member_order_by_name() {
        let (pool, db_path) = setup_test_db().await;
        let json_path = temp_json_path();

        // Source project: three repos, a profile with two of them in a specific order.
        let source = persistence::upsert_project(&pool, "Source", "/repos/source")
            .await
            .expect("upsert source failed");
        let api = persistence::upsert_repository(&pool, source.id, "api", "/repos/source/api", "npm", Some("dev"), true)
            .await
            .expect("upsert api failed");
        let worker = persistence::upsert_repository(&pool, source.id, "worker", "/repos/source/worker", "npm", Some("dev"), true)
            .await
            .expect("upsert worker failed");
        persistence::upsert_repository(&pool, source.id, "web", "/repos/source/web", "npm", Some("dev"), true)
            .await
            .expect("upsert web failed");

        let profile_id = persistence::create_profile(&pool, source.id, "Backend", 2500)
            .await
            .expect("create profile failed");
        persistence::set_profile_members(&pool, profile_id, &[worker.id, api.id])
            .await
            .expect("set members failed");

        export_profile_impl(&pool, profile_id, json_path.to_str().unwrap())
            .await
            .expect("export failed");

        // Target project: same repo NAMES, but different database ids and paths (a different
        // machine's discovery of the same project) -- this is what import must re-match against.
        let target = persistence::upsert_project(&pool, "Target", "/repos/target")
            .await
            .expect("upsert target failed");
        let target_worker = persistence::upsert_repository(&pool, target.id, "worker", "/repos/target/worker", "npm", Some("dev"), true)
            .await
            .expect("upsert target worker failed");
        let target_api = persistence::upsert_repository(&pool, target.id, "api", "/repos/target/api", "npm", Some("dev"), true)
            .await
            .expect("upsert target api failed");
        assert_ne!(target_worker.id, worker.id, "sanity check: ids must differ across projects");

        let result = import_profile_impl(&pool, target.id, json_path.to_str().unwrap())
            .await
            .expect("import failed");

        assert_eq!(result.profile.name, "Backend");
        assert_eq!(result.profile.launch_delay_ms, 2500);
        assert_eq!(
            result.profile.repository_ids,
            vec![target_worker.id, target_api.id],
            "members must re-match by name to the TARGET project's own ids, in the exported order"
        );
        assert!(result.skipped_members.is_empty());

        let _ = std::fs::remove_file(&json_path);
        pool.close().await;
        cleanup_db(&db_path);
    }

    #[tokio::test]
    async fn import_reports_partially_skipped_members_but_succeeds() {
        let (pool, db_path) = setup_test_db().await;
        let json_path = temp_json_path();

        let source = persistence::upsert_project(&pool, "Source", "/repos/source2")
            .await
            .expect("upsert source failed");
        let api = persistence::upsert_repository(&pool, source.id, "api", "/repos/source2/api", "npm", Some("dev"), true)
            .await
            .expect("upsert api failed");
        let web = persistence::upsert_repository(&pool, source.id, "web", "/repos/source2/web", "npm", Some("dev"), true)
            .await
            .expect("upsert web failed");
        let profile_id = persistence::create_profile(&pool, source.id, "Backend", 1000)
            .await
            .expect("create profile failed");
        persistence::set_profile_members(&pool, profile_id, &[api.id, web.id])
            .await
            .expect("set members failed");
        export_profile_impl(&pool, profile_id, json_path.to_str().unwrap())
            .await
            .expect("export failed");
        // "web" has no matching repository in the target project (renamed/removed there), but
        // "api" does -- a partial mismatch, distinct from the "wrong project entirely" case.
        let mut contents = std::fs::read_to_string(&json_path).unwrap();
        contents = contents.replace("\"web\"", "\"ghost\"");
        std::fs::write(&json_path, contents).unwrap();

        let target = persistence::upsert_project(&pool, "Target", "/repos/target2")
            .await
            .expect("upsert target failed");
        let target_api = persistence::upsert_repository(&pool, target.id, "api", "/repos/target2/api", "npm", Some("dev"), true)
            .await
            .expect("upsert target api failed");

        let result = import_profile_impl(&pool, target.id, json_path.to_str().unwrap())
            .await
            .expect("import should succeed when at least one member matches");

        assert_eq!(result.skipped_members, vec!["ghost".to_string()]);
        assert_eq!(result.profile.repository_ids, vec![target_api.id]);

        let _ = std::fs::remove_file(&json_path);
        pool.close().await;
        cleanup_db(&db_path);
    }

    #[tokio::test]
    async fn import_rejects_when_no_members_match_target_project() {
        let (pool, db_path) = setup_test_db().await;
        let json_path = temp_json_path();

        let source = persistence::upsert_project(&pool, "Source", "/repos/source4")
            .await
            .expect("upsert source failed");
        let api = persistence::upsert_repository(&pool, source.id, "api", "/repos/source4/api", "npm", Some("dev"), true)
            .await
            .expect("upsert api failed");
        let profile_id = persistence::create_profile(&pool, source.id, "Backend", 1000)
            .await
            .expect("create profile failed");
        persistence::set_profile_members(&pool, profile_id, &[api.id])
            .await
            .expect("set members failed");
        export_profile_impl(&pool, profile_id, json_path.to_str().unwrap())
            .await
            .expect("export failed");

        // Target project shares NOTHING by name with the source -- e.g. a completely different
        // project, like importing a multi-repo profile into an unrelated monorepo fixture.
        let target = persistence::upsert_project(&pool, "Target", "/repos/target4")
            .await
            .expect("upsert target failed");
        persistence::upsert_repository(&pool, target.id, "web", "/repos/target4/web", "npm", Some("dev"), true)
            .await
            .expect("upsert target web failed");

        let err = import_profile_impl(&pool, target.id, json_path.to_str().unwrap())
            .await
            .expect_err("import must be rejected when nothing matches");
        assert!(
            err.to_string().contains("different project"),
            "error should explain why it was rejected, got: {err}"
        );

        // No profile must have been created for the rejected import.
        let profiles_after = persistence::list_profiles(&pool, target.id)
            .await
            .expect("list profiles failed");
        assert!(profiles_after.is_empty(), "rejected import must not create a profile");

        let _ = std::fs::remove_file(&json_path);
        pool.close().await;
        cleanup_db(&db_path);
    }

    #[tokio::test]
    async fn import_auto_suffixes_name_on_collision() {
        let (pool, db_path) = setup_test_db().await;
        let json_path = temp_json_path();

        let source = persistence::upsert_project(&pool, "Source", "/repos/source3")
            .await
            .expect("upsert source failed");
        let profile_id = persistence::create_profile(&pool, source.id, "Backend", 1000)
            .await
            .expect("create profile failed");
        export_profile_impl(&pool, profile_id, json_path.to_str().unwrap())
            .await
            .expect("export failed");

        let target = persistence::upsert_project(&pool, "Target", "/repos/target3")
            .await
            .expect("upsert target failed");
        persistence::create_profile(&pool, target.id, "Backend", 500)
            .await
            .expect("pre-existing Backend profile failed");

        let first = import_profile_impl(&pool, target.id, json_path.to_str().unwrap())
            .await
            .expect("first import failed");
        assert_eq!(first.profile.name, "Backend (imported)");

        let second = import_profile_impl(&pool, target.id, json_path.to_str().unwrap())
            .await
            .expect("second import failed");
        assert_eq!(second.profile.name, "Backend (imported 2)");

        let _ = std::fs::remove_file(&json_path);
        pool.close().await;
        cleanup_db(&db_path);
    }
}

#[cfg(test)]
mod list_env_files_tests {
    use super::*;

    async fn setup_test_db() -> (sqlx::SqlitePool, std::path::PathBuf) {
        let unique = format!(
            "mrl_envfiles_test_{}_{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let db_path = std::env::temp_dir().join(unique);
        let pool = persistence::init_pool(&db_path).await.expect("init_pool failed");
        persistence::run_migrations(&pool).await.expect("run_migrations failed");
        (pool, db_path)
    }

    fn cleanup_db(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    fn temp_repo_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mrl-envfiles-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create repo dir");
        dir
    }

    #[tokio::test]
    async fn lists_only_dotenv_style_files_sorted_and_ignores_the_rest() {
        let (pool, db_path) = setup_test_db().await;
        let repo_dir = temp_repo_dir();

        std::fs::write(repo_dir.join(".env.production"), "").unwrap();
        std::fs::write(repo_dir.join(".env"), "").unwrap();
        std::fs::write(repo_dir.join(".env.local"), "").unwrap();
        std::fs::write(repo_dir.join("package.json"), "{}").unwrap();
        std::fs::write(repo_dir.join("README.md"), "").unwrap();
        // A directory whose NAME matches the pattern too - must still be excluded by the is_file() check.
        std::fs::create_dir_all(repo_dir.join(".env.test")).unwrap();

        let project = persistence::upsert_project(&pool, "Proj", repo_dir.to_str().unwrap())
            .await
            .expect("upsert project failed");
        let repo = persistence::upsert_repository(
            &pool,
            project.id,
            "api",
            repo_dir.to_str().unwrap(),
            "npm",
            Some("dev"),
            true,
        )
        .await
        .expect("upsert repository failed");

        let files = list_env_files_impl(&pool, repo.id).await.expect("list_env_files_impl failed");

        assert_eq!(files, vec![".env", ".env.local", ".env.production"]);

        let _ = std::fs::remove_dir_all(&repo_dir);
        pool.close().await;
        cleanup_db(&db_path);
    }

    #[tokio::test]
    async fn returns_empty_list_when_repository_folder_has_no_env_files() {
        let (pool, db_path) = setup_test_db().await;
        let repo_dir = temp_repo_dir();
        std::fs::write(repo_dir.join("package.json"), "{}").unwrap();

        let project = persistence::upsert_project(&pool, "Proj", repo_dir.to_str().unwrap())
            .await
            .expect("upsert project failed");
        let repo = persistence::upsert_repository(
            &pool,
            project.id,
            "api",
            repo_dir.to_str().unwrap(),
            "npm",
            Some("dev"),
            true,
        )
        .await
        .expect("upsert repository failed");

        let files = list_env_files_impl(&pool, repo.id).await.expect("list_env_files_impl failed");
        assert!(files.is_empty());

        let _ = std::fs::remove_dir_all(&repo_dir);
        pool.close().await;
        cleanup_db(&db_path);
    }
}
