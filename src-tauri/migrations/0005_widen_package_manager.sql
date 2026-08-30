-- Widen repositories.package_manager to accept non-Node toolchains (v0.2.0 item 2B):
-- Python (pip/poetry/uv/pipenv), Rust (cargo), .NET (dotnet), alongside the existing
-- npm/pnpm/yarn/bun. Patches the stored CHECK constraint text in place rather than
-- rebuilding the table, since SQLite has no ALTER TABLE for CHECK constraints and a
-- rebuild would need to re-point three child tables' foreign keys inside this migration's
-- transaction.
PRAGMA writable_schema = ON;

UPDATE sqlite_schema
SET sql = REPLACE(
    sql,
    'CHECK (package_manager IN (''npm'', ''pnpm'', ''yarn'', ''bun''))',
    'CHECK (package_manager IN (''npm'', ''pnpm'', ''yarn'', ''bun'', ''pip'', ''poetry'', ''uv'', ''pipenv'', ''cargo'', ''dotnet''))'
)
WHERE type = 'table' AND name = 'repositories';

PRAGMA writable_schema = OFF;
