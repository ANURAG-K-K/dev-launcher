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

/// Open an external terminal at the repository's working directory (F12). An in-app
/// "integrated" terminal is deferred past v1 (R6) — this always opens an external window, in
/// the shell chosen by `Settings.terminal_shell`.
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
        let mut args = vec!["/C", "start", "", shell];
        if shell == "powershell" {
            // Same execution-policy bypass as windows_launch_program_args — otherwise the
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
            // "canonical" here means the repository's DB path used verbatim as the cache key —
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
        // consistent with the existing "non-git repos are omitted" contract — not a bug, a deliberate choice.
        if let Ok(Some(status)) = handle.await {
            out.push(status);
        }
    }
    Ok(out)
}

/// Builds a `git -C <path>` command with console-window suppression on Windows (every git
/// subprocess in this app must be spawned through this helper — see spec 2026-07-31).
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

    let dirty = git_command(path)
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

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

/// Fetch from the repository's remote (R5). Invalidates the cached status for this repo —
/// any git write command must do the same (spec 2026-07-31).
#[tauri::command]
pub async fn git_fetch(state: State<'_, AppState>, repository_id: i64) -> Result<String, AppError> {
    let repo = git_repo_path(&state, repository_id).await?;
    let result = run_git(&repo, &["fetch"]);
    state.git_status_cache.invalidate(repository_id);
    result
}

/// Fast-forward pull the repository (R5). `--ff-only` avoids merge prompts/conflicts hanging.
/// Invalidates the cached status for this repo — any git write command must do the same.
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
                // yarn/bun install (npm.ps1 etc.) — without this, every launch fails with
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

const SETTINGS_KEY: &str = "app_settings";

/// Loads `Settings` from the `app_settings` blob, falling back to `Settings::default()` if
/// missing or unparseable — the one place every settings read goes through.
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
        // needs a full AppState/sqlite pool — this exercises the same cache + git_status_for path
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
