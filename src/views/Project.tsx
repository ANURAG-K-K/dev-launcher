import { useState } from "react";
import { scanRepositories } from "@/api";
import type { Project as ProjectT, ProjectWithRepos, Repository } from "@/types";

/** Project view (F4): repository list for the open project + Refresh (F2/D3). */
export function Project({
  project,
  repositories,
  onScanned,
}: {
  project: ProjectT | null;
  repositories: Repository[];
  onScanned: (result: ProjectWithRepos) => void;
}) {
  const [busy, setBusy] = useState(false);

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
                <th>Repository</th>
                <th>Package Manager</th>
                <th>Command</th>
                <th>Enabled</th>
              </tr>
            </thead>
            <tbody>
              {repositories.map((r) => (
                <tr key={r.id}>
                  <td style={{ fontWeight: 600 }}>{r.name}</td>
                  <td>
                    <span className="tag tag-neutral">{r.packageManager}</span>
                  </td>
                  <td className="mono text-muted" style={{ fontSize: 12 }}>
                    {r.command ?? r.detectedScript ?? (
                      <span style={{ color: "var(--color-status-bad-fg)" }}>no script detected</span>
                    )}
                  </td>
                  <td>
                    {r.enabled ? (
                      <span className="tag tag-good">● Enabled</span>
                    ) : (
                      <span className="tag tag-neutral">○ Disabled</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
