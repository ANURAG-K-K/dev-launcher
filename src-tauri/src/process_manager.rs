//! Process manager (F7/F9/F10, ADR-0003 / D2; detailed design §2).
//!
//! Owns the lifecycle of every spawned dev-server process: spawn via `tokio::process`,
//! stream stdout/stderr line-by-line to the frontend via Tauri events, track
//! PID/status/exit code/restart count, and tree-kill on Stop/Kill/Restart.
//!
//! On Windows the child is assigned to a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
//! so the entire process tree dies together (no orphaned `node` children); `taskkill /T /F`
//! is the fallback path.
//!
//! TODO: implement spawn/stop/restart/kill_all and Job Object assignment (adds the `windows`
//! crate to Cargo.toml).
