//! Repository scanner (F2/F3, architecture §3.1; detailed design §1).
//!
//! Scan-on-demand only (D3): enumerate the project root's immediate child directories,
//! classify a repo as a dir containing `package.json`, detect the package manager from the
//! lockfile (`pnpm-lock.yaml` → pnpm, `package-lock.json` → npm, `yarn.lock` → yarn,
//! `bun.lock` → bun), and recommend a startup script by priority
//! (`start:dev` → `dev` → `start` → `serve` → `watch`).

use std::path::Path;

/// Package manager detected for a discovered repository, by lockfile presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
        }
    }
}

/// A repository discovered under a project root (design.md §1.5).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredRepo {
    pub name: String,
    pub path: String,
    pub package_manager: PackageManager,
    pub detected_script: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("project root not found or not a directory: {0}")]
    RootNotADirectory(String),
    #[error("failed to read directory: {0}")]
    Io(String),
}

/// Lockfile → package manager precedence, first match wins (design.md §1.2 / SRS FR-5).
const LOCKFILE_PRECEDENCE: &[(&str, PackageManager)] = &[
    ("pnpm-lock.yaml", PackageManager::Pnpm),
    ("package-lock.json", PackageManager::Npm),
    ("yarn.lock", PackageManager::Yarn),
    ("bun.lock", PackageManager::Bun),
];

/// Script keys searched in priority order; first one present in `scripts` wins.
const SCRIPT_PRIORITY: &[&str] = &["start:dev", "dev", "start", "serve", "watch"];

fn detect_package_manager(repo_dir: &Path) -> PackageManager {
    for (lockfile, manager) in LOCKFILE_PRECEDENCE {
        if repo_dir.join(lockfile).is_file() {
            return *manager;
        }
    }
    PackageManager::Npm
}

/// Parses `package.json` and returns the first known script key present in `scripts`.
/// Malformed JSON or a missing/non-object `scripts` field yields `None` rather than an error
/// (design.md §1.3/§1.5 — a bad package.json must not fail the whole scan).
fn detect_script(pkg_json_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(pkg_json_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let scripts = json.get("scripts")?.as_object()?;
    SCRIPT_PRIORITY
        .iter()
        .find(|&&candidate| scripts.contains_key(candidate))
        .map(|&s| s.to_string())
}

/// Builds the default display/launch command for a manager + script (design.md §1.4).
fn build_command(manager: PackageManager, script: &str) -> String {
    match manager {
        PackageManager::Npm => format!("npm run {script}"),
        PackageManager::Pnpm => format!("pnpm run {script}"),
        PackageManager::Bun => format!("bun run {script}"),
        PackageManager::Yarn => format!("yarn {script}"),
    }
}

/// Scans the immediate child directories of `root` (non-recursive) and returns the discovered
/// repositories, sorted by `name` ascending. A child directory is a repository iff it directly
/// contains `package.json`; other children are skipped silently.
pub fn scan_project_root(root: &Path) -> Result<Vec<DiscoveredRepo>, ScanError> {
    if !root.is_dir() {
        return Err(ScanError::RootNotADirectory(root.display().to_string()));
    }

    let entries =
        std::fs::read_dir(root).map_err(|e| ScanError::Io(format!("{}: {e}", root.display())))?;

    let mut repos = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ScanError::Io(format!("{}: {e}", root.display())))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let pkg_json = path.join("package.json");
        if !pkg_json.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let package_manager = detect_package_manager(&path);
        let detected_script = detect_script(&pkg_json);
        let command = detected_script
            .as_deref()
            .map(|script| build_command(package_manager, script));

        repos.push(DiscoveredRepo {
            name,
            path: path.display().to_string(),
            package_manager,
            detected_script,
            command,
        });
    }

    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(repos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Creates a unique temp directory under the OS temp dir, cleaned up via `Drop`.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "mrl-scanner-test-{}-{}-{}",
                std::process::id(),
                n,
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ));
            std::fs::create_dir_all(&dir).expect("create temp root");
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn child_dir(&self, name: &str) -> std::path::PathBuf {
            let p = self.0.join(name);
            std::fs::create_dir_all(&p).expect("create child dir");
            p
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, name: &str, contents: &str) {
        std::fs::write(path.join(name), contents).expect("write fixture file");
    }

    #[test]
    fn dir_without_package_json_is_ignored() {
        let tmp = TempDir::new();
        tmp.child_dir("not-a-repo");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn lockfile_precedence_multi_lockfile_prefers_npm_over_yarn() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("repo-a");
        write(&repo, "package.json", r#"{"scripts": {"start": "node ."}}"#);
        write(&repo, "yarn.lock", "");
        write(&repo, "package-lock.json", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].package_manager, PackageManager::Npm);
    }

    #[test]
    fn lockfile_precedence_pnpm_wins_over_all() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("repo-b");
        write(&repo, "package.json", "{}");
        write(&repo, "pnpm-lock.yaml", "");
        write(&repo, "package-lock.json", "");
        write(&repo, "yarn.lock", "");
        write(&repo, "bun.lock", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].package_manager, PackageManager::Pnpm);
    }

    #[test]
    fn no_lockfile_defaults_to_npm() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("repo-c");
        write(&repo, "package.json", "{}");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].package_manager, PackageManager::Npm);
    }

    #[test]
    fn script_priority_prefers_dev_over_start_when_no_start_dev() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("repo-d");
        write(
            &repo,
            "package.json",
            r#"{"scripts": {"dev": "vite", "start": "node ."}}"#,
        );

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].detected_script.as_deref(), Some("dev"));
    }

    #[test]
    fn no_known_script_yields_none_command_and_script() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("repo-e");
        write(&repo, "package.json", r#"{"scripts": {"build": "tsc"}}"#);

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].detected_script, None);
        assert_eq!(repos[0].command, None);
    }

    #[test]
    fn malformed_package_json_treated_as_no_script_and_scan_succeeds() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("repo-f");
        write(&repo, "package.json", "{ not valid json ");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].detected_script, None);
        assert_eq!(repos[0].command, None);
    }

    #[test]
    fn command_templates_and_yarn_omits_run() {
        assert_eq!(
            build_command(PackageManager::Npm, "dev"),
            "npm run dev"
        );
        assert_eq!(
            build_command(PackageManager::Pnpm, "start:dev"),
            "pnpm run start:dev"
        );
        assert_eq!(
            build_command(PackageManager::Bun, "dev"),
            "bun run dev"
        );
        assert_eq!(build_command(PackageManager::Yarn, "dev"), "yarn dev");
    }

    #[test]
    fn nonexistent_root_returns_root_not_a_directory() {
        let tmp = TempDir::new();
        let missing = tmp.path().join("does-not-exist");

        let err = scan_project_root(&missing).unwrap_err();
        assert!(matches!(err, ScanError::RootNotADirectory(_)));
    }

    #[test]
    fn results_sorted_by_name_ascending() {
        let tmp = TempDir::new();
        for name in ["zeta", "alpha", "mid"] {
            let repo = tmp.child_dir(name);
            write(&repo, "package.json", "{}");
        }

        let repos = scan_project_root(tmp.path()).unwrap();
        let names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
    }
}
