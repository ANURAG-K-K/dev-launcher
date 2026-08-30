import { useEffect, useState, type CSSProperties } from "react";
import { listEnvFiles, pickDirectory, pickEnvFilePath, setRepositoryDependencies, updateRepositoryConfig, updateRepositoryPath } from "@/api";
import type { DependencyEdge, Repository } from "@/types";

const INPUT_STYLE: CSSProperties = {
  width: "100%",
  padding: "8px 10px",
  border: "1px solid var(--color-divider)",
  background: "var(--color-bg)",
  borderRadius: "var(--radius-md)",
};

const PACKAGE_MANAGERS = ["npm", "pnpm", "yarn", "bun"] as const;

function Field({
  label,
  helper,
  children,
}: {
  label: string;
  helper?: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 16 }}>
      <label className="sectiontitle" style={{ display: "block", marginBottom: 6 }}>
        {label}
      </label>
      {children}
      {helper && (
        <div className="text-muted" style={{ fontSize: 11, marginTop: 4 }}>
          {helper}
        </div>
      )}
    </div>
  );
}

/** Modal dialog to edit per-repository launch config (package manager, command, args, env file). */
export function RepoEditDialog({
  repo,
  repositories,
  dependencies,
  onClose,
  onRepoUpdated,
  onDependenciesChanged,
}: {
  repo: Repository;
  repositories: Repository[];
  dependencies: DependencyEdge[];
  onClose: () => void;
  onRepoUpdated: (repo: Repository) => void;
  onDependenciesChanged: () => void;
}) {
  const [packageManager, setPackageManager] = useState(repo.packageManager);
  const [command, setCommand] = useState(repo.command ?? "");
  const [args, setArgs] = useState(repo.args ?? "");
  const [envFile, setEnvFile] = useState(repo.envFile ?? "");
  const [envFiles, setEnvFiles] = useState<string[]>([]);
  const [envMode, setEnvMode] = useState<"none" | "discovered" | "custom">(repo.envFile ? "custom" : "none");
  const [dependsOn, setDependsOn] = useState<number[]>(
    dependencies.filter((d) => d.repositoryId === repo.id).map((d) => d.dependsOnRepositoryId),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [path, setPath] = useState(repo.path);
  const [name, setName] = useState(repo.name);
  const [pathBusy, setPathBusy] = useState(false);
  const [pathError, setPathError] = useState("");

  const otherRepos = repositories.filter((r) => r.id !== repo.id);

  useEffect(() => {
    listEnvFiles(repo.id)
      .then((files) => {
        setEnvFiles(files);
        if (repo.envFile && files.includes(repo.envFile)) {
          setEnvMode("discovered");
        }
      })
      .catch(() => {
        // Best-effort - the dropdown just stays empty; Custom path still works.
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo.id]);

  function toggleDependsOn(id: number, checked: boolean) {
    setDependsOn((prev) => (checked ? [...prev, id] : prev.filter((x) => x !== id)));
  }

  async function browsePath() {
    const dir = await pickDirectory();
    if (!dir) return;
    setPathError("");
    setPathBusy(true);
    try {
      const updated = await updateRepositoryPath(repo.id, dir);
      setPath(updated.path);
      setName(updated.name);
      setPackageManager(updated.packageManager);
      onRepoUpdated(updated);
    } catch (err) {
      setPathError(String(err));
    } finally {
      setPathBusy(false);
    }
  }

  async function save() {
    setBusy(true);
    setError("");
    try {
      const updated = await updateRepositoryConfig({
        repositoryId: repo.id,
        packageManager,
        command,
        args,
        envFile,
      });
      await setRepositoryDependencies(repo.id, dependsOn);
      onRepoUpdated(updated);
      onDependenciesChanged();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.4)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
      }}
      onClick={onClose}
    >
      <div
        className="card"
        style={{ width: "100%", maxWidth: 480, background: "var(--color-bg)", padding: 24 }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 style={{ fontSize: 18, marginBottom: 18 }}>Edit — {name}</h2>

        <Field label="Path" helper="Folder this service launches from.">
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <span
              className="mono text-muted"
              style={{ fontSize: 12, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}
            >
              {path}
            </span>
            <button type="button" className="btn btn-secondary" onClick={browsePath} disabled={pathBusy}>
              {pathBusy ? "Checking…" : "Browse…"}
            </button>
          </div>
          {pathError && (
            <div style={{ color: "var(--color-status-bad-fg)", fontSize: 11, marginTop: 4 }}>{pathError}</div>
          )}
        </Field>

        <Field label="Package manager">
          <select
            className="mono"
            style={INPUT_STYLE}
            value={packageManager}
            onChange={(e) => setPackageManager(e.target.value)}
          >
            {PACKAGE_MANAGERS.map((pm) => (
              <option key={pm} value={pm}>
                {pm}
              </option>
            ))}
          </select>
        </Field>

        <Field label="Command" helper="Full command to run. Leave empty to use the detected script.">
          <input
            className="mono"
            style={INPUT_STYLE}
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder={
              repo.detectedScript ? `${repo.packageManager} run ${repo.detectedScript} (detected)` : "e.g. pnpm tauri dev"
            }
          />
        </Field>

        <Field label="Args" helper="Extra arguments appended to the command.">
          <input
            className="mono"
            style={INPUT_STYLE}
            value={args}
            onChange={(e) => setArgs(e.target.value)}
          />
        </Field>

        <Field label="Env file" helper="Pick one detected in this service's folder, or choose Custom path for one elsewhere.">
          <select
            className="mono"
            style={INPUT_STYLE}
            value={envMode === "custom" ? "__custom__" : envMode === "discovered" ? envFile : ""}
            onChange={(e) => {
              const v = e.target.value;
              if (v === "__custom__") {
                setEnvMode("custom");
              } else if (v === "") {
                setEnvMode("none");
                setEnvFile("");
              } else {
                setEnvMode("discovered");
                setEnvFile(v);
              }
            }}
          >
            <option value="">(None)</option>
            {envFiles.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
            <option value="__custom__">Custom path…</option>
          </select>
          {envMode === "custom" && (
            <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8 }}>
              <input
                className="mono"
                style={{ ...INPUT_STYLE, flex: 1 }}
                value={envFile}
                onChange={(e) => setEnvFile(e.target.value)}
                placeholder="Absolute or relative path"
              />
              <button
                type="button"
                className="btn btn-secondary"
                onClick={async () => {
                  const picked = await pickEnvFilePath();
                  if (picked) setEnvFile(picked);
                }}
              >
                Browse…
              </button>
            </div>
          )}
        </Field>

        <Field label="Depends on" helper="Services that must be started before this one.">
          {otherRepos.length === 0 ? (
            <div className="text-muted" style={{ fontSize: 12 }}>No other services in this project.</div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 6, maxHeight: 160, overflowY: "auto" }}>
              {otherRepos.map((r) => (
                <label key={r.id} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
                  <input
                    type="checkbox"
                    checked={dependsOn.includes(r.id)}
                    onChange={(e) => toggleDependsOn(r.id, e.target.checked)}
                  />
                  {r.name}
                </label>
              ))}
            </div>
          )}
        </Field>

        {error && (
          <div style={{ color: "var(--color-status-bad-fg)", fontSize: 12, marginBottom: 12 }}>
            {error}
          </div>
        )}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 8 }}>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary" onClick={save} disabled={busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
