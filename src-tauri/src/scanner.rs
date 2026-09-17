//! Repository scanner (F2/F3, architecture §3.1; detailed design §1).
//!
//! Scan-on-demand only (D3): enumerate the project root's immediate child directories,
//! classify a repo as a dir containing `package.json`, detect the package manager from the
//! lockfile (`pnpm-lock.yaml` → pnpm, `package-lock.json` → npm, `yarn.lock` → yarn,
//! `bun.lock` → bun), and recommend a startup script by priority
//! (`start:dev` → `dev` → `start` → `serve` → `watch`).
//!
//! A child directory that is itself not a repo (no recognized project marker) is checked one level
//! deeper for repos nested inside it (`parent -> sub_dir -> repo`), so a grouping folder such
//! as `services/` doesn't hide the repos underneath it. Nested repos are named
//! `sub_dir/repo` to disambiguate same-named repos under different groups. Nothing past that
//! one extra level is scanned - a repo the auto-scan still doesn't find can be added manually
//! (`classify_repo_dir`, used by the `add_repository_manual` command).

use std::path::Path;

/// Toolchain detected for a discovered repository, by lockfile/marker presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Pip,
    Poetry,
    Uv,
    Pipenv,
    Cargo,
    Dotnet,
}

impl PackageManager {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
            PackageManager::Pip => "pip",
            PackageManager::Poetry => "poetry",
            PackageManager::Uv => "uv",
            PackageManager::Pipenv => "pipenv",
            PackageManager::Cargo => "cargo",
            PackageManager::Dotnet => "dotnet",
        }
    }

    /// Parses a persisted `package_manager` string back into its enum value. Used by
    /// `commands.rs` to regenerate the default command for a repository row (design.md
    /// §4) and to validate `update_repository_config`'s input.
    pub fn parse(s: &str) -> Option<PackageManager> {
        match s {
            "npm" => Some(PackageManager::Npm),
            "pnpm" => Some(PackageManager::Pnpm),
            "yarn" => Some(PackageManager::Yarn),
            "bun" => Some(PackageManager::Bun),
            "pip" => Some(PackageManager::Pip),
            "poetry" => Some(PackageManager::Poetry),
            "uv" => Some(PackageManager::Uv),
            "pipenv" => Some(PackageManager::Pipenv),
            "cargo" => Some(PackageManager::Cargo),
            "dotnet" => Some(PackageManager::Dotnet),
            _ => None,
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

/// One ecosystem's detection rules: how to recognize a repo of this kind, how to pick its
/// toolchain, and how to build its default launch command. `ECOSYSTEMS` below is checked in
/// order; the first `marker` match wins (design.md §2 - deliberate, not a heuristic to
/// refine later).
struct EcosystemSpec {
    /// True if `dir` is a repository root for this ecosystem.
    marker: fn(&Path) -> bool,
    /// Lockfile -> toolchain, checked in order; first match wins. Empty for
    /// single-toolchain ecosystems (Rust, .NET).
    toolchain_precedence: &'static [(&'static str, PackageManager)],
    /// Toolchain when no entry in `toolchain_precedence` matches (or the list is empty).
    default_toolchain: PackageManager,
}

fn has_marker_file(dir: &Path, files: &[&str]) -> bool {
    files.iter().any(|f| dir.join(f).is_file())
}

fn has_file_with_extension(dir: &Path, ext: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|e| {
        let path = e.path();
        path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|found| found.eq_ignore_ascii_case(ext))
    })
}

const ECOSYSTEMS: &[EcosystemSpec] = &[
    // Node
    EcosystemSpec {
        marker: |dir| dir.join("package.json").is_file(),
        toolchain_precedence: &[
            ("pnpm-lock.yaml", PackageManager::Pnpm),
            ("package-lock.json", PackageManager::Npm),
            ("yarn.lock", PackageManager::Yarn),
            ("bun.lock", PackageManager::Bun),
        ],
        default_toolchain: PackageManager::Npm,
    },
    // Python
    EcosystemSpec {
        marker: |dir| has_marker_file(dir, &["pyproject.toml", "requirements.txt", "Pipfile"]),
        toolchain_precedence: &[
            ("poetry.lock", PackageManager::Poetry),
            ("uv.lock", PackageManager::Uv),
            ("Pipfile.lock", PackageManager::Pipenv),
        ],
        default_toolchain: PackageManager::Pip,
    },
    // Rust
    EcosystemSpec {
        marker: |dir| dir.join("Cargo.toml").is_file(),
        toolchain_precedence: &[],
        default_toolchain: PackageManager::Cargo,
    },
    // .NET
    EcosystemSpec {
        marker: |dir| has_file_with_extension(dir, "csproj") || has_file_with_extension(dir, "sln"),
        toolchain_precedence: &[],
        default_toolchain: PackageManager::Dotnet,
    },
];

fn detect_toolchain(dir: &Path, spec: &EcosystemSpec) -> PackageManager {
    for (lockfile, manager) in spec.toolchain_precedence {
        if dir.join(lockfile).is_file() {
            return *manager;
        }
    }
    spec.default_toolchain
}

/// Script keys searched in priority order for Node repos; first one present in `scripts` wins.
/// Only Node has a `scripts` field to read - every other ecosystem always has `detected_script
/// == None` and relies entirely on `default_command`'s fixed default (design.md §3).
const SCRIPT_PRIORITY: &[&str] = &["start:dev", "dev", "start", "serve", "watch"];

/// Parses `package.json` and returns the first known script key present in `scripts`.
/// Malformed JSON or a missing/non-object `scripts` field yields `None` rather than an error
/// (design.md §1.3/§1.5 - a bad package.json must not fail the whole scan).
fn detect_node_script(pkg_json_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(pkg_json_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let scripts = json.get("scripts")?.as_object()?;
    SCRIPT_PRIORITY
        .iter()
        .find(|&&candidate| scripts.contains_key(candidate))
        .map(|&s| s.to_string())
}

/// Builds the default launch command for a toolchain. `detected_script` is only ever `Some`
/// for Node (the only ecosystem with a script list to read); every other toolchain ignores it
/// and returns its one fixed default (design.md §3). This is the single source of truth for
/// "what does this repository run by default" - called both at scan time (`classify_repo_dir`)
/// and at launch/filter time in `commands.rs`, so the two never drift apart (design.md §4).
pub fn default_command(pm: PackageManager, detected_script: Option<&str>) -> Option<String> {
    match pm {
        PackageManager::Npm => detected_script.map(|s| format!("npm run {s}")),
        PackageManager::Pnpm => detected_script.map(|s| format!("pnpm run {s}")),
        PackageManager::Bun => detected_script.map(|s| format!("bun run {s}")),
        PackageManager::Yarn => detected_script.map(|s| format!("yarn {s}")),
        PackageManager::Pip => Some("python main.py".to_string()),
        PackageManager::Poetry => Some("poetry run python main.py".to_string()),
        PackageManager::Uv => Some("uv run main.py".to_string()),
        PackageManager::Pipenv => Some("pipenv run python main.py".to_string()),
        PackageManager::Cargo => Some("cargo run".to_string()),
        PackageManager::Dotnet => Some("dotnet run".to_string()),
    }
}

/// Classifies a single directory as a repository iff it matches some ecosystem's marker
/// (design.md §2, first match in `ECOSYSTEMS` order wins). `display_name` overrides the leaf
/// directory name (used to qualify nested repos as `sub_dir/repo`); pass `None` to use the
/// directory's own name.
pub fn classify_repo_dir(path: &Path, display_name: Option<&str>) -> Option<DiscoveredRepo> {
    let spec = ECOSYSTEMS.iter().find(|spec| (spec.marker)(path))?;

    let name = match display_name {
        Some(n) => n.to_string(),
        None => path.file_name()?.to_string_lossy().into_owned(),
    };
    let package_manager = detect_toolchain(path, spec);
    // `detect_node_script` is already a no-op (returns `None`) for any directory without a
    // `package.json` - which is exactly every directory that reached a later ecosystem's spec,
    // since Node's own marker is checked first and didn't match. So this doesn't need to know
    // which ecosystem matched; no index/enumerate bookkeeping required.
    let detected_script = detect_node_script(&path.join("package.json"));
    let command = default_command(package_manager, detected_script.as_deref());

    Some(DiscoveredRepo {
        name,
        path: path.display().to_string(),
        package_manager,
        detected_script,
        command,
    })
}

/// Non-dot subdirectories of `dir`. Unreadable entries/dirs are skipped rather than failing
/// the scan (this is only ever used for the best-effort nested lookup, not the project root).
fn subdirs(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(true)
        })
        .collect()
}

/// Scans the project root and returns the discovered repositories, sorted by `name` ascending.
/// Checks immediate child directories, plus one extra level under any child that is itself not
/// a repo (`parent -> sub_dir -> repo`, design.md §1.5 / feedback #8).
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
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue; // skip .git, .superpowers, etc. before any metadata/package.json check
        }

        if let Some(repo) = classify_repo_dir(&path, None) {
            repos.push(repo);
            continue;
        }
        for nested in subdirs(&path) {
            let display_name = nested
                .file_name()
                .map(|leaf| format!("{name}/{}", leaf.to_string_lossy()));
            if let Some(repo) = classify_repo_dir(&nested, display_name.as_deref()) {
                repos.push(repo);
            }
        }
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
        assert_eq!(default_command(PackageManager::Npm, Some("dev")).as_deref(), Some("npm run dev"));
        assert_eq!(default_command(PackageManager::Pnpm, Some("start:dev")).as_deref(), Some("pnpm run start:dev"));
        assert_eq!(default_command(PackageManager::Bun, Some("dev")).as_deref(), Some("bun run dev"));
        assert_eq!(default_command(PackageManager::Yarn, Some("dev")).as_deref(), Some("yarn dev"));
    }

    #[test]
    fn python_repo_detected_via_pyproject_toml() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "pyproject.toml", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].package_manager, PackageManager::Pip);
        assert_eq!(repos[0].command.as_deref(), Some("python main.py"));
    }

    #[test]
    fn python_repo_detected_via_requirements_txt() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "requirements.txt", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].package_manager, PackageManager::Pip);
    }

    #[test]
    fn python_repo_detected_via_pipfile() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "Pipfile", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].package_manager, PackageManager::Pip);
    }

    #[test]
    fn python_toolchain_precedence_poetry_beats_uv() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "pyproject.toml", "");
        write(&repo, "poetry.lock", "");
        write(&repo, "uv.lock", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].package_manager, PackageManager::Poetry);
        assert_eq!(repos[0].command.as_deref(), Some("poetry run python main.py"));
    }

    #[test]
    fn python_toolchain_uv_alone() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "pyproject.toml", "");
        write(&repo, "uv.lock", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].package_manager, PackageManager::Uv);
        assert_eq!(repos[0].command.as_deref(), Some("uv run main.py"));
    }

    #[test]
    fn python_toolchain_pipenv_alone() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "Pipfile", "");
        write(&repo, "Pipfile.lock", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].package_manager, PackageManager::Pipenv);
        assert_eq!(repos[0].command.as_deref(), Some("pipenv run python main.py"));
    }

    #[test]
    fn python_no_lockfile_defaults_to_pip() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "requirements.txt", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos[0].package_manager, PackageManager::Pip);
        assert_eq!(repos[0].command.as_deref(), Some("python main.py"));
    }

    #[test]
    fn rust_repo_detected_via_cargo_toml() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "Cargo.toml", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].package_manager, PackageManager::Cargo);
        assert_eq!(repos[0].command.as_deref(), Some("cargo run"));
    }

    #[test]
    fn dotnet_repo_detected_via_csproj() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "MyApp.csproj", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].package_manager, PackageManager::Dotnet);
        assert_eq!(repos[0].command.as_deref(), Some("dotnet run"));
    }

    #[test]
    fn dotnet_repo_detected_via_sln() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "MyApp.sln", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].package_manager, PackageManager::Dotnet);
    }

    #[test]
    fn dotnet_marker_matching_is_case_insensitive() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "MyApp.CSPROJ", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1, "an uppercase .CSPROJ extension must still match the .csproj marker");
        assert_eq!(repos[0].package_manager, PackageManager::Dotnet);
    }

    #[test]
    fn directory_named_like_a_marker_file_is_not_mistaken_for_one() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        // A subdirectory whose name happens to end in ".csproj" - not a file, so it must not
        // satisfy the .NET marker check on its own.
        std::fs::create_dir_all(repo.join("fake.csproj")).expect("create decoy directory");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert!(
            repos.is_empty(),
            "a directory merely named like a marker file must not be treated as a .NET repo: {repos:?}"
        );
    }

    #[test]
    fn ecosystem_precedence_node_wins_over_rust_when_both_markers_present() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("svc");
        write(&repo, "package.json", "{}");
        write(&repo, "Cargo.toml", "");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert!(matches!(
            repos[0].package_manager,
            PackageManager::Npm | PackageManager::Pnpm | PackageManager::Yarn | PackageManager::Bun
        ));
    }

    #[test]
    fn no_ecosystem_marker_is_not_detected() {
        let tmp = TempDir::new();
        tmp.child_dir("not-a-repo-of-any-kind");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn default_command_covers_every_new_toolchain() {
        assert_eq!(default_command(PackageManager::Pip, None).as_deref(), Some("python main.py"));
        assert_eq!(default_command(PackageManager::Poetry, None).as_deref(), Some("poetry run python main.py"));
        assert_eq!(default_command(PackageManager::Uv, None).as_deref(), Some("uv run main.py"));
        assert_eq!(default_command(PackageManager::Pipenv, None).as_deref(), Some("pipenv run python main.py"));
        assert_eq!(default_command(PackageManager::Cargo, None).as_deref(), Some("cargo run"));
        assert_eq!(default_command(PackageManager::Dotnet, None).as_deref(), Some("dotnet run"));
    }

    #[test]
    fn default_command_node_still_requires_a_script() {
        assert_eq!(default_command(PackageManager::Npm, Some("dev")).as_deref(), Some("npm run dev"));
        assert_eq!(default_command(PackageManager::Yarn, Some("dev")).as_deref(), Some("yarn dev"));
        assert_eq!(default_command(PackageManager::Npm, None), None);
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

    #[test]
    fn nested_repo_one_level_under_non_repo_parent_is_found_and_qualified() {
        let tmp = TempDir::new();
        let group = tmp.child_dir("services");
        let repo = group.join("api");
        std::fs::create_dir_all(&repo).unwrap();
        write(&repo, "package.json", "{}");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "services/api");
    }

    #[test]
    fn repo_directory_is_not_recursed_into_for_nested_repos() {
        let tmp = TempDir::new();
        let repo = tmp.child_dir("top-repo");
        write(&repo, "package.json", "{}");
        let nested = repo.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        write(&nested, "package.json", "{}");

        let repos = scan_project_root(tmp.path()).unwrap();
        let names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["top-repo"]);
    }

    #[test]
    fn two_levels_deep_is_not_found() {
        let tmp = TempDir::new();
        let group = tmp.child_dir("a");
        let subgroup = group.join("b");
        let repo = subgroup.join("c");
        std::fs::create_dir_all(&repo).unwrap();
        write(&repo, "package.json", "{}");

        let repos = scan_project_root(tmp.path()).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn dot_prefixed_directory_is_skipped_even_if_it_somehow_contains_package_json() {
        let tmp = TempDir::new();
        let dotdir = tmp.child_dir(".git");
        write(&dotdir, "package.json", "{}"); // pathological case: prove it's skipped explicitly
        let repo = tmp.child_dir("real-repo");
        write(&repo, "package.json", "{}");

        let repos = scan_project_root(tmp.path()).unwrap();
        let names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["real-repo"]);
    }

    #[test]
    fn package_manager_as_str_covers_every_variant() {
        assert_eq!(PackageManager::Pip.as_str(), "pip");
        assert_eq!(PackageManager::Poetry.as_str(), "poetry");
        assert_eq!(PackageManager::Uv.as_str(), "uv");
        assert_eq!(PackageManager::Pipenv.as_str(), "pipenv");
        assert_eq!(PackageManager::Cargo.as_str(), "cargo");
        assert_eq!(PackageManager::Dotnet.as_str(), "dotnet");
    }

    #[test]
    fn package_manager_parse_round_trips_every_as_str_value() {
        let all = [
            PackageManager::Npm,
            PackageManager::Pnpm,
            PackageManager::Yarn,
            PackageManager::Bun,
            PackageManager::Pip,
            PackageManager::Poetry,
            PackageManager::Uv,
            PackageManager::Pipenv,
            PackageManager::Cargo,
            PackageManager::Dotnet,
        ];
        for pm in all {
            assert_eq!(PackageManager::parse(pm.as_str()), Some(pm));
        }
    }

    #[test]
    fn package_manager_parse_rejects_unknown_string() {
        assert_eq!(PackageManager::parse("not-a-real-toolchain"), None);
    }
}
