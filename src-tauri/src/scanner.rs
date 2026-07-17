//! Repository scanner (F2/F3, architecture §3.1; detailed design §1).
//!
//! Scan-on-demand only (D3): enumerate the project root's immediate child directories,
//! classify a repo as a dir containing `package.json`, detect the package manager from the
//! lockfile (`pnpm-lock.yaml` → pnpm, `package-lock.json` → npm, `yarn.lock` → yarn,
//! `bun.lock` → bun), and recommend a startup script by priority
//! (`start:dev` → `dev` → `start` → `serve` → `watch`).
//!
//! TODO: implement `scan_project_root(root: &Path) -> Result<Vec<DiscoveredRepo>, AppError>`.
