import { useEffect, useState } from "react";
import { listRecentProjects, openProject, pickDirectory } from "@/api";
import type { Project, ProjectWithRepos } from "@/types";

/** Home view (F1): recent projects + Open Project Folder. */
export function Home({ onOpened }: { onOpened: (result: ProjectWithRepos) => void }) {
  const [recent, setRecent] = useState<Project[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listRecentProjects().then(setRecent).catch((e) => setError(String(e)));
  }, []);

  async function open(rootPath: string) {
    setBusy(true);
    setError(null);
    try {
      onOpened(await openProject(rootPath));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function chooseFolder() {
    const dir = await pickDirectory();
    if (dir) await open(dir);
  }

  return (
    <div style={{ maxWidth: 920, padding: "40px 48px" }}>
      <h1 style={{ fontSize: 32, marginBottom: 6 }}>Home</h1>
      <p className="text-muted" style={{ margin: "0 0 28px", maxWidth: "56ch" }}>
        Open a project root to discover repositories, or jump back into one you had running.
      </p>

      <div className="card elev-sm" style={{ marginBottom: 24 }}>
        <div className="card-kicker">Open Project</div>
        <div className="card-title">Select a project root folder</div>
        <div style={{ marginTop: 14 }}>
          <button type="button" className="btn btn-primary" onClick={chooseFolder} disabled={busy}>
            {busy ? "Opening…" : "Open Project Folder"}
          </button>
        </div>
      </div>

      {error && (
        <p style={{ color: "var(--color-status-bad-fg)", marginBottom: 24 }}>{error}</p>
      )}

      <div className="hr" style={{ margin: "0 0 24px" }} />

      <div className="sectiontitle" style={{ padding: "0 0 10px" }}>Recent</div>
      {recent.length === 0 ? (
        <p className="text-muted">No recent projects yet.</p>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>Project</th>
              <th>Path</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {recent.map((p) => (
              <tr key={p.id}>
                <td style={{ fontWeight: 600 }}>{p.name}</td>
                <td className="mono text-muted" style={{ fontSize: 12 }}>{p.rootPath}</td>
                <td style={{ textAlign: "right" }}>
                  <button type="button" className="btn btn-secondary" onClick={() => open(p.rootPath)} disabled={busy}>
                    Open
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
