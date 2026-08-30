//! Process manager (F7/F9/F10, ADR-0003 / D2; detailed design §2).
//!
//! Spawns dev-server processes with `tokio::process`, streams stdout/stderr to the frontend via
//! `repo_log` Tauri events, tracks live status, and - on Windows - assigns each process to a
//! Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so Stop/Restart/quit terminate the whole
//! process tree (no orphaned `node` children). v1 uses a forced Job Object kill (no graceful
//! `CTRL_BREAK` phase - see docs open question C1).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};

/// Live process status, serialized lowercase to match the frontend contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcStatus {
    Starting,
    Running,
    Stopped,
    Crashed,
}

/// `repo_status_changed` event payload / command return.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdate {
    pub repository_id: i64,
    pub status: ProcStatus,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub restart_count: u32,
}

/// `repo_log` event payload.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogLine {
    repository_id: i64,
    stream: &'static str,
    line: String,
    timestamp: String,
}

/// What to run for a repository (resolved by the command layer).
#[derive(Clone)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    /// Extra environment variables (from the repo's env file, F5).
    pub env: Vec<(String, String)>,
    /// When true, spawn in a real visible console window with inherited stdio (interactive
    /// keyboard access) instead of a hidden window with piped output. Trade-off: no `repo_log`
    /// events are emitted for a visible-console run - Windows can't both pipe stdio for capture
    /// and hand it to a console window at the same time.
    pub visible: bool,
}

struct Tracked {
    status: ProcStatus,
    pid: Option<u32>,
    exit_code: Option<i32>,
    restart_count: u32,
    /// Bumped on every (re)spawn so a stale exit-watcher can't clobber a newer process's state.
    generation: u64,
    /// Set true on a user-initiated stop so the exit watcher records Stopped rather than Crashed.
    stopping: Arc<AtomicBool>,
    spec: LaunchSpec,
    /// The launch_history_items row to mirror this process's exit into (F13), if part of a run.
    history_item_id: Option<i64>,
    /// Windows Job Object handle as an isize (0 = none). isize is Send, unlike a raw HANDLE.
    #[cfg(windows)]
    job: isize,
}

#[derive(Clone)]
pub struct ProcessManager {
    app: AppHandle,
    pool: sqlx::SqlitePool,
    procs: Arc<Mutex<HashMap<i64, Tracked>>>,
    generation: Arc<AtomicU64>,
}

impl ProcessManager {
    pub fn new(app: AppHandle, pool: sqlx::SqlitePool) -> Self {
        Self {
            app,
            pool,
            procs: Arc::new(Mutex::new(HashMap::new())),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Start a repository's process. Errors if it is already running/starting.
    /// `history_item_id` (when part of an Execute All run) receives the terminal exit mirror (F13).
    pub async fn start(
        &self,
        repo_id: i64,
        spec: LaunchSpec,
        history_item_id: Option<i64>,
    ) -> Result<StatusUpdate, String> {
        let restart_count = {
            let map = self.procs.lock().unwrap();
            if let Some(t) = map.get(&repo_id) {
                if matches!(t.status, ProcStatus::Running | ProcStatus::Starting) {
                    return Err("repository is already running".into());
                }
                t.restart_count
            } else {
                0
            }
        };
        self.spawn(repo_id, spec, restart_count, history_item_id)
    }

    /// Stop a repository's process tree (forced). The exit watcher emits the terminal status.
    pub fn stop(&self, repo_id: i64) -> Result<(), String> {
        let map = self.procs.lock().unwrap();
        let t = map
            .get(&repo_id)
            .ok_or_else(|| "repository is not running".to_string())?;
        if !matches!(t.status, ProcStatus::Running | ProcStatus::Starting) {
            return Err("repository is not running".into());
        }
        t.stopping.store(true, Ordering::SeqCst);
        #[cfg(windows)]
        unsafe {
            terminate_job(t.job);
        }
        #[cfg(not(windows))]
        {
            return Err("process control is only implemented on Windows (v1)".into());
        }
        #[cfg(windows)]
        Ok(())
    }

    /// Whether a repository is currently running or starting, per the live tracker (not the
    /// database). Used by `delete_project` to guard against deleting a project out from under a
    /// process still writing to it.
    pub fn is_running(&self, repo_id: i64) -> bool {
        let map = self.procs.lock().unwrap();
        matches!(
            map.get(&repo_id).map(|t| t.status),
            Some(ProcStatus::Running | ProcStatus::Starting)
        )
    }

    /// Stop then re-spawn, incrementing the restart count.
    pub async fn restart(&self, repo_id: i64) -> Result<StatusUpdate, String> {
        let spec = {
            let map = self.procs.lock().unwrap();
            map.get(&repo_id)
                .map(|t| t.spec.clone())
                .ok_or_else(|| "repository is not running".to_string())?
        };
        self.stop(repo_id)?;
        // Give the Job Object kill a moment to reap the tree before re-spawning.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let restart_count = {
            let map = self.procs.lock().unwrap();
            map.get(&repo_id).map(|t| t.restart_count).unwrap_or(0)
        } + 1;
        self.spawn(repo_id, spec, restart_count, None)
    }

    /// Kill every tracked process tree (app-quit cleanup - no orphans).
    pub fn kill_all(&self) {
        let map = self.procs.lock().unwrap();
        for t in map.values() {
            t.stopping.store(true, Ordering::SeqCst);
            #[cfg(windows)]
            unsafe {
                terminate_job(t.job);
            }
        }
    }

    fn spawn(
        &self,
        repo_id: i64,
        spec: LaunchSpec,
        restart_count: u32,
        history_item_id: Option<i64>,
    ) -> Result<StatusUpdate, String> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let stopping = Arc::new(AtomicBool::new(false));

        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args)
            .current_dir(&spec.cwd)
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .kill_on_drop(false);
        if spec.visible {
            // Inherited stdio goes to the new console window, giving real keyboard access - but
            // that means it can't also be piped, so no repo_log events for this run (see LaunchSpec).
            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            #[cfg(windows)]
            cmd.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
        } else {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: output is captured via pipes.
        }

        let mut child = cmd.spawn().map_err(|e| format!("failed to spawn: {e}"))?;
        let pid = child.id();

        #[cfg(windows)]
        let job = unsafe { assign_to_job(&child) }.unwrap_or(0);

        if spec.visible {
            let payload = LogLine {
                repository_id: repo_id,
                stream: "stdout",
                line: "- running in a visible console window; output is not captured here -".into(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            let _ = self.app.emit("repo_log", &payload);
        } else {
            if let Some(out) = child.stdout.take() {
                self.stream_output(repo_id, out, "stdout");
            }
            if let Some(err) = child.stderr.take() {
                self.stream_output(repo_id, err, "stderr");
            }
        }

        {
            let mut map = self.procs.lock().unwrap();
            map.insert(
                repo_id,
                Tracked {
                    status: ProcStatus::Running,
                    pid,
                    exit_code: None,
                    restart_count,
                    generation,
                    stopping: stopping.clone(),
                    spec,
                    history_item_id,
                    #[cfg(windows)]
                    job,
                },
            );
        }

        let update = StatusUpdate {
            repository_id: repo_id,
            status: ProcStatus::Running,
            pid,
            exit_code: None,
            restart_count,
        };
        let _ = self.app.emit("repo_status_changed", &update);

        // Exit watcher: waits for the process to exit, then records the terminal status
        // (unless a newer generation has already replaced this entry).
        let procs = self.procs.clone();
        let app = self.app.clone();
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let exit = child.wait().await;
            let code = exit.ok().and_then(|s| s.code());
            let was_stopping = stopping.load(Ordering::SeqCst);

            let mut final_update = None;
            {
                let mut map = procs.lock().unwrap();
                if let Some(t) = map.get_mut(&repo_id) {
                    if t.generation != generation {
                        return; // a newer spawn owns this repo now.
                    }
                    t.status = if was_stopping {
                        ProcStatus::Stopped
                    } else {
                        ProcStatus::Crashed
                    };
                    t.exit_code = code;
                    #[cfg(windows)]
                    unsafe {
                        close_job(t.job);
                        t.job = 0;
                    }
                    final_update = Some(StatusUpdate {
                        repository_id: repo_id,
                        status: t.status,
                        pid: t.pid,
                        exit_code: code,
                        restart_count: t.restart_count,
                    });
                }
            }
            if let Some(u) = final_update {
                let _ = app.emit("repo_status_changed", &u);
                // Mirror the terminal state into launch history (F13), if part of a run.
                if let Some(item_id) = history_item_id {
                    let status = if was_stopping { "stopped" } else { "crashed" };
                    let now = chrono::Utc::now().to_rfc3339();
                    let _ = crate::persistence::update_launch_history_item_exit(
                        &pool,
                        item_id,
                        status,
                        code.map(|c| c as i64),
                        &now,
                    )
                    .await;
                }
            }
        });

        Ok(update)
    }

    fn stream_output<R>(&self, repo_id: i64, reader: R, stream: &'static str)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let app = self.app.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let payload = LogLine {
                    repository_id: repo_id,
                    stream,
                    line,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                let _ = app.emit("repo_log", &payload);
            }
        });
    }
}

// ── Windows Job Object helpers ─────────────────────────────────────────────
// A Job Object created with KILL_ON_JOB_CLOSE terminates every process assigned to it (and their
// children) when the job is terminated/closed - this is how a Stop reliably kills the whole
// `cmd → pnpm → node` tree instead of leaking orphaned children.

#[cfg(windows)]
unsafe fn assign_to_job(child: &tokio::process::Child) -> Option<isize> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let process_handle = child.raw_handle()?;
    let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
    if job.is_null() {
        return None;
    }

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    SetInformationJobObject(
        job,
        JobObjectExtendedLimitInformation,
        &info as *const _ as *const core::ffi::c_void,
        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
    );
    AssignProcessToJobObject(job, process_handle as _);
    Some(job as isize)
}

#[cfg(windows)]
unsafe fn terminate_job(job: isize) {
    if job != 0 {
        windows_sys::Win32::System::JobObjects::TerminateJobObject(job as _, 1);
    }
}

#[cfg(windows)]
unsafe fn close_job(job: isize) {
    if job != 0 {
        windows_sys::Win32::Foundation::CloseHandle(job as _);
    }
}
