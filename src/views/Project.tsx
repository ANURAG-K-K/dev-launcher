import { useState, type CSSProperties, type ReactNode } from "react";
import { restartRepo, scanRepositories, setRepositoryEnabled, startRepo, stopRepo } from "@/api";
import type { Project as ProjectT, ProjectWithRepos, RepoStatus, Repository } from "@/types";

const ICON_BTN: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  padding: 6,
  cursor: "pointer",
  background: "transparent",
  border: "none",
  color: "var(--color-text)",
  borderRadius: 4,
};

function IconButton({
  title,
  onClick,
  disabled,
  color,
  children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  color?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      disabled={disabled}
      style={{ ...ICON_BTN, color: color ?? ICON_BTN.color, opacity: disabled ? 0.4 : 1 }}
      onMouseEnter={(e) => {
        if (!disabled) e.currentTarget.style.background = "color-mix(in srgb, var(--color-text) 8%, transparent)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        {children}
      </svg>
    </button>
  );
}

/** Project view (F4): repository list for the open project + Refresh (F2/D3). */
export function Project({
  project,
  repositories,
  statuses,
  onScanned,
  onOpenLogs,
  onRepoUpdated,
}: {
  project: ProjectT | null;
  repositories: Repository[];
  statuses: Record<number, RepoStatus>;
  onScanned: (result: ProjectWithRepos) => void;
  onOpenLogs: (repositoryId: number) => void;
  onRepoUpdated: (repo: Repository) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [rowBusy, setRowBusy] = useState<number | null>(null);
  const [rowError, setRowError] = useState<Record<number, string>>({});
  const [enabledBusy, setEnabledBusy] = useState<number | null>(null);

  if (!project) {
    return (
      <div style={{ padding: "40px 48px" }}>
        <p className="text-muted">Open a project from the Home screen first.</p>
      </div>
    );
  }

  async function refresh() {
    setBusy(true);
    try {
      onScanned(await scanRepositories(project!.id));
    } finally {
      setBusy(false);
    }
  }

  async function runAction(repositoryId: number, action: () => Promise<unknown>) {
    setRowBusy(repositoryId);
    setRowError((prev) => ({ ...prev, [repositoryId]: "" }));
    try {
      await action();
    } catch (err) {
      setRowError((prev) => ({ ...prev, [repositoryId]: String(err) }));
    } finally {
      setRowBusy(null);
    }
  }

  async function toggleEnabled(repo: Repository, enabled: boolean) {
    setEnabledBusy(repo.id);
    setRowError((prev) => ({ ...prev, [repo.id]: "" }));
    try {
      const updated = await setRepositoryEnabled(repo.id, enabled);
      onRepoUpdated(updated);
    } catch (err) {
      setRowError((prev) => ({ ...prev, [repo.id]: String(err) }));
    } finally {
      setEnabledBusy(null);
    }
  }

  return (
    <div style={{ padding: "40px 48px 64px" }}>
      <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 16, flexWrap: "wrap", marginBottom: 6 }}>
        <h1 style={{ fontSize: 32 }}>{project.name}</h1>
        <div className="mono text-muted" style={{ fontSize: 12 }}>{project.rootPath}</div>
      </div>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 16, flexWrap: "wrap", marginBottom: 22 }}>
        <p className="text-muted" style={{ margin: 0 }}>
          {repositories.length} {repositories.length === 1 ? "repository" : "repositories"} discovered
        </p>
        <button type="button" className="btn btn-secondary" onClick={refresh} disabled={busy}>
          {busy ? "Scanning…" : "Refresh"}
        </button>
      </div>

      {repositories.length === 0 ? (
        <p className="text-muted">No repositories with a package.json found under this root.</p>
      ) : (
        <div style={{ overflowX: "auto" }}>
          <table className="table" style={{ minWidth: 640 }}>
            <thead>
              <tr>
                <th>Enabled</th>
                <th>Repository</th>
                <th>PID</th>
                <th>Package Manager</th>
                <th>Command</th>
                <th>Status</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {repositories.map((r) => {
                const repoStatus = statuses[r.id];
                const status = repoStatus?.status ?? "stopped";
                const launchable = r.command != null || r.detectedScript != null;
                const busyRow = rowBusy === r.id;
                const error = rowError[r.id];
                const running = status === "running" || status === "starting";
                return (
                  <tr key={r.id}>
                    <td>
                      <input
                        type="checkbox"
                        checked={r.enabled === 1}
                        disabled={enabledBusy === r.id}
                        onChange={(e) => toggleEnabled(r, e.target.checked)}
                      />
                    </td>
                    <td style={{ fontWeight: 600 }}>{r.name}</td>
                    <td className="mono text-muted" style={{ fontSize: 12 }}>
                      {running && repoStatus?.pid != null ? repoStatus.pid : "—"}
                    </td>
                    <td>
                      <span className="tag tag-neutral">{r.packageManager}</span>
                    </td>
                    <td className="mono text-muted" style={{ fontSize: 12 }}>
                      {r.command ?? r.detectedScript ?? (
                        <span style={{ color: "var(--color-status-bad-fg)" }}>no script detected</span>
                      )}
                    </td>
                    <td>
                      {status === "running" && <span className="tag tag-good">● Running</span>}
                      {status === "crashed" && <span className="tag tag-bad">● Crashed</span>}
                      {status === "starting" && <span className="tag tag-neutral">◌ Starting</span>}
                      {status === "stopped" && <span className="tag tag-neutral">○ Stopped</span>}
                    </td>
                    <td>
                      <div style={{ display: "flex", alignItems: "center", gap: 4, flexWrap: "wrap" }}>
                        {(status === "stopped" || status === "crashed") && (
                          <IconButton
                            title="Start"
                            disabled={!launchable || busyRow}
                            onClick={() => runAction(r.id, () => startRepo(r.id))}
                          >
                            <path d="M6 3l14 9-14 9V3z" />
                          </IconButton>
                        )}
                        {(status === "running" || status === "starting") && (
                          <IconButton
                            title="Stop"
                            color="var(--color-status-bad-fg)"
                            disabled={busyRow}
                            onClick={() => runAction(r.id, () => stopRepo(r.id))}
                          >
                            <rect x="5" y="5" width="14" height="14" rx="1" />
                          </IconButton>
                        )}
                        {status === "running" && (
                          <IconButton
                            title="Restart"
                            disabled={busyRow}
                            onClick={() => runAction(r.id, () => restartRepo(r.id))}
                          >
                            <path d="M21 12a9 9 0 1 1-3-6.7" />
                            <path d="M21 3v6h-6" />
                          </IconButton>
                        )}
                        <IconButton title="Logs" onClick={() => onOpenLogs(r.id)}>
                          <path d="M4 6h16M4 12h16M4 18h10" />
                        </IconButton>
                      </div>
                      {error && (
                        <div style={{ color: "var(--color-status-bad-fg)", fontSize: 11, marginTop: 4 }}>
                          {error}
                        </div>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
