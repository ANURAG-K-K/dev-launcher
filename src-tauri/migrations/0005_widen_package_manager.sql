-- Widen repositories.package_manager to accept non-Node toolchains (v0.2.0 item 2B):
-- Python (pip/poetry/uv/pipenv), Rust (cargo), .NET (dotnet), alongside the existing
-- npm/pnpm/yarn/bun.
--
-- SQLite has no ALTER TABLE ... ALTER COLUMN / DROP CONSTRAINT, so a CHECK constraint
-- change requires the standard SQLite table-rebuild procedure (see "Making Other Kinds
-- Of Table Schema Changes" in the SQLite ALTER TABLE docs): create a replacement table
-- with the desired schema, copy the data across, drop the original, then rename the
-- replacement into place.
--
-- THE HAZARD THIS FILE WORKS AROUND: `repository_dependencies`, `profile_repositories`,
-- and `launch_history_items` all hold `ON DELETE CASCADE` foreign keys to
-- repositories(id). Our connections always run with `PRAGMA foreign_keys = ON` (see
-- `persistence::init_pool`). With FK enforcement on, `DROP TABLE repositories` makes
-- SQLite perform an *implicit row-by-row DELETE* of the table being dropped before
-- removing it -- and that implicit delete fires the `ON DELETE CASCADE` action on all
-- three dependent tables, wiping every row in them (every dependency edge, every
-- profile's repo membership, all launch history) for every project, not just the rows
-- touched by this migration.
--
-- The standard fix for this is `PRAGMA foreign_keys = OFF` around the rebuild -- but
-- that pragma is a documented no-op when changed mid-transaction, and sqlx's SQLite
-- migrator *always* wraps a whole migration file in one transaction (confirmed by
-- reading `Sqlite::apply` in sqlx-sqlite 0.8.6 -- it calls `self.begin()` unconditionally
-- with no opt-out for this driver, unlike some other sqlx backends). `PRAGMA
-- defer_foreign_keys = ON` does NOT help either: it only defers constraint *checks*, not
-- cascade *actions*, which fire synchronously as part of the DELETE itself.
--
-- So instead: snapshot the three dependent tables into temp tables before the rebuild
-- (temp tables carry no FK metadata, so they're unaffected by the cascade), then restore
-- their exact rows afterwards. Every id involved (repositories.id and the dependent
-- tables' own ids) is preserved unchanged throughout, so this is a true no-op for
-- existing data once the transaction commits.
CREATE TEMP TABLE _2b_repository_dependencies_backup AS SELECT * FROM repository_dependencies;
CREATE TEMP TABLE _2b_profile_repositories_backup AS SELECT * FROM profile_repositories;
CREATE TEMP TABLE _2b_launch_history_items_backup AS SELECT * FROM launch_history_items;

CREATE TABLE repositories_new (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id        INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    path              TEXT NOT NULL,
    package_manager   TEXT NOT NULL CHECK (package_manager IN (
                          'npm', 'pnpm', 'yarn', 'bun',
                          'pip', 'poetry', 'uv', 'pipenv',
                          'cargo', 'dotnet'
                      )),
    detected_script   TEXT,
    command           TEXT,
    args              TEXT,
    env_file          TEXT,
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    removed_at        TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    favorite          INTEGER NOT NULL DEFAULT 0,
    visible_console   INTEGER NOT NULL DEFAULT 0,
    UNIQUE (project_id, path)
);

INSERT INTO repositories_new (
    id, project_id, name, path, package_manager, detected_script, command, args,
    env_file, enabled, removed_at, created_at, updated_at, favorite, visible_console
)
SELECT
    id, project_id, name, path, package_manager, detected_script, command, args,
    env_file, enabled, removed_at, created_at, updated_at, favorite, visible_console
FROM repositories;

-- Cascades: wipes repository_dependencies, profile_repositories, and launch_history_items
-- rows (see comment above). Restored from the temp-table snapshots below.
DROP TABLE repositories;

ALTER TABLE repositories_new RENAME TO repositories;

CREATE INDEX idx_repositories_project_id ON repositories(project_id);

INSERT INTO repository_dependencies SELECT * FROM _2b_repository_dependencies_backup;
INSERT INTO profile_repositories SELECT * FROM _2b_profile_repositories_backup;
INSERT INTO launch_history_items SELECT * FROM _2b_launch_history_items_backup;

DROP TABLE _2b_repository_dependencies_backup;
DROP TABLE _2b_profile_repositories_backup;
DROP TABLE _2b_launch_history_items_backup;
