import { useState, type CSSProperties } from "react";
import { updateRepositoryConfig } from "@/api";
import type { Repository } from "@/types";

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
  onClose,
  onRepoUpdated,
}: {
  repo: Repository;
  onClose: () => void;
  onRepoUpdated: (repo: Repository) => void;
}) {
  const [packageManager, setPackageManager] = useState(repo.packageManager);
  const [command, setCommand] = useState(repo.command ?? "");
  const [args, setArgs] = useState(repo.args ?? "");
  const [envFile, setEnvFile] = useState(repo.envFile ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

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
      onRepoUpdated(updated);
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
        <h2 style={{ fontSize: 18, marginBottom: 18 }}>Edit — {repo.name}</h2>

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

        <Field label="Env file" helper="Path to a .env file (absolute or relative to the repo).">
          <input
            className="mono"
            style={INPUT_STYLE}
            value={envFile}
            onChange={(e) => setEnvFile(e.target.value)}
          />
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
