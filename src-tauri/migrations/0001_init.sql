-- Multi-Repo Dev Launcher — initial schema (v1).
-- Mirrors docs/data-model.md. Rust-owned via sqlx (D1). Forward-only migration.

PRAGMA foreign_keys = ON;

CREATE TABLE projects (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT NOT NULL,
    root_path        TEXT NOT NULL UNIQUE,
    last_opened_at   TEXT NOT NULL,
    last_profile_id  INTEGER REFERENCES profiles(id) ON DELETE SET NULL,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);
CREATE INDEX idx_projects_last_opened_at ON projects(last_opened_at);

CREATE TABLE repositories (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id        INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    path              TEXT NOT NULL,
    package_manager   TEXT NOT NULL CHECK (package_manager IN ('npm', 'pnpm', 'yarn', 'bun')),
    detected_script   TEXT,
    command           TEXT,
    args              TEXT,
    env_file          TEXT,
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    removed_at        TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE (project_id, path)
);
CREATE INDEX idx_repositories_project_id ON repositories(project_id);

CREATE TABLE repository_dependencies (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id               INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    depends_on_repository_id    INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    UNIQUE (repository_id, depends_on_repository_id),
    CHECK (repository_id != depends_on_repository_id)
);
CREATE INDEX idx_repo_deps_repository_id ON repository_dependencies(repository_id);
CREATE INDEX idx_repo_deps_depends_on_id ON repository_dependencies(depends_on_repository_id);

CREATE TABLE profiles (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id        INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    launch_delay_ms   INTEGER NOT NULL DEFAULT 1000,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE (project_id, name)
);
CREATE INDEX idx_profiles_project_id ON profiles(project_id);

CREATE TABLE profile_repositories (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id          INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    repository_id       INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    enabled             INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    launch_order        INTEGER NOT NULL,
    command_override    TEXT,
    args_override       TEXT,
    env_override        TEXT,
    UNIQUE (profile_id, repository_id)
);
CREATE INDEX idx_profile_repos_profile_id ON profile_repositories(profile_id);
CREATE INDEX idx_profile_repos_repository_id ON profile_repositories(repository_id);

CREATE TABLE launch_history (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id     INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    profile_id     INTEGER REFERENCES profiles(id) ON DELETE SET NULL,
    started_at     TEXT NOT NULL,
    finished_at    TEXT,
    status         TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'completed', 'failed'))
);
CREATE INDEX idx_launch_history_project_started ON launch_history(project_id, started_at);

CREATE TABLE launch_history_items (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    launch_history_id    INTEGER NOT NULL REFERENCES launch_history(id) ON DELETE CASCADE,
    repository_id        INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    pid                  INTEGER,
    status               TEXT NOT NULL DEFAULT 'pending'
                             CHECK (status IN ('pending', 'running', 'stopped', 'crashed', 'restarting')),
    exit_code            INTEGER,
    restart_count        INTEGER NOT NULL DEFAULT 0,
    started_at           TEXT,
    stopped_at           TEXT
);
CREATE INDEX idx_launch_items_launch_history_id ON launch_history_items(launch_history_id);
CREATE INDEX idx_launch_items_repository_id ON launch_history_items(repository_id);

CREATE TABLE settings (
    key      TEXT PRIMARY KEY,
    value    TEXT NOT NULL
);
