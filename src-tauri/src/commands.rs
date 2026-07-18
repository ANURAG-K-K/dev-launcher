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

    let mut started = Vec::new();
    for (i, id) in order.iter().enumerate() {
        let spec = launch_spec_for(&state, *id).await?;
        if state.process_manager.start(*id, spec).await.is_ok() {
            started.push(*id);
        }
        if i + 1 < order.len() {
            tokio::time::sleep(std::time::Duration::from_millis(launch_delay_ms)).await;
        }
    }

    let skipped: Vec<i64> = enabled_ids
        .into_iter()
        .filter(|id| !started.contains(id))
        .collect();
    Ok(ExecuteResult { started, skipped })
}

/// Stop every running repository in a project (F9 — Stop All). Repos that aren't running are
/// ignored.
#[tauri::command]
pub async fn stop_all(state: State<'_, AppState>, project_id: i64) -> Result<(), AppError> {
    let repos = persistence::list_repositories(&state.pool, project_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?;
    for repo in repos {
        // Ignore "not running" errors — we only care that nothing is left running.
        let _ = state.process_manager.stop(repo.id);
    }
    Ok(())
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
        .start(repository_id, spec)
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
/// `bun`) are `.cmd` shims, so we run through `cmd /C <…>` — this is exactly why the Job Object
/// tree-kill (ADR-0003) is required.
async fn launch_spec_for(
    state: &AppState,
    repository_id: i64,
) -> Result<LaunchSpec, AppError> {
    let repo = persistence::get_repository(&state.pool, repository_id)
        .await
        .map_err(|e| AppError::Persist(e.to_string()))?
        .ok_or_else(|| AppError::Launch(format!("repository {repository_id} not found")))?;

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

    #[cfg(windows)]
    {
        Ok(LaunchSpec {
            program: "cmd".to_string(),
            args: vec!["/C".to_string(), command_line],
            cwd: repo.path,
            env,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(LaunchSpec {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), command_line],
            cwd: repo.path,
            env,
        })
    }
}

/// Parse a simple `.env` file (KEY=VALUE per line; `#` comments and blanks ignored; surrounding
/// double quotes on the value stripped). `env_file` may be absolute or relative to `repo_path`.
/// ponytail: naive parser — no multiline/escape handling; upgrade to a dotenv crate if needed.
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
