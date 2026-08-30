import { useEffect, useRef, useState } from "react";
import { deleteProject, listRecentProjects, openProject, pickDirectory, renameProject, updateProjectPath } from "@/api";
import type { Project, ProjectWithRepos } from "@/types";
import { IconButton } from "@/components/IconButton";

/** Home view (F1): recent projects + Open Project Folder. */
export function Home({
  onOpened,
  onRenamed,
  onPathUpdated,
  onDeleted,
}: {
  onOpened: (result: ProjectWithRepos) => void;
  onRenamed?: (project: Project) => void;
  onPathUpdated: (result: ProjectWithRepos) => void;
  onDeleted: (projectId: number) => void;
}) {
  const [recent, setRecent] = useState<Project[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameError, setRenameError] = useState("");
  const renameCancelledRef = useRef(false);
  const [pathBusyId, setPathBusyId] = useState<number | null>(null);
  const [deleteBusyId, setDeleteBusyId] = useState<number | null>(null);
  const [rowError, setRowError] = useState<Record<number, string>>({});

  useEffect(() => {
    listRecentProjects().then(setRecent).catch((e) => setError(String(e)));
  }, []);

  function startRename(p: Project) {
    // Cleared unconditionally on every edit-start (not just consumed-and-reset on the blur
    // path in saveRename): correctness can't depend on the browser firing `blur` when a
    // focused element unmounts - Chromium/WebView2 (this app's actual runtime) doesn't.
    renameCancelledRef.current = false;
    setRenamingId(p.id);
    setRenameValue(p.name);
    setRenameError("");
  }

  function cancelRename() {
    renameCancelledRef.current = true;
    setRenamingId(null);
    setRenameError("");
  }

  async function saveRename(id: number) {
    if (renameCancelledRef.current) {
      renameCancelledRef.current = false;
      return;
    }
    const trimmed = renameValue.trim();
    if (!trimmed) {
      setRenameError("Name cannot be empty");
      return;
    }
    try {
      const updated = await renameProject(id, trimmed);
      setRecent((prev) => prev.map((p) => (p.id === id ? updated : p)));
      onRenamed?.(updated);
      setRenamingId(null);
      setRenameError("");
    } catch (err) {
      setRenameError(String(err));
    }
  }

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

  async function editPath(p: Project) {
    const dir = await pickDirectory();
    if (!dir) return;
    setRowError((prev) => ({ ...prev, [p.id]: "" }));
    setPathBusyId(p.id);
    try {
      const result = await updateProjectPath(p.id, dir);
      setRecent((prev) => prev.map((x) => (x.id === result.project.id ? result.project : x)));
      onPathUpdated(result);
    } catch (err) {
      setRowError((prev) => ({ ...prev, [p.id]: String(err) }));
    } finally {
      setPathBusyId(null);
    }
  }

  async function deleteProjectRow(p: Project) {
    if (!confirm(`Delete project "${p.name}"? This removes it and all its service configuration permanently.`)) {
      return;
    }
    setRowError((prev) => ({ ...prev, [p.id]: "" }));
    setDeleteBusyId(p.id);
    try {
      await deleteProject(p.id);
      setRecent((prev) => prev.filter((x) => x.id !== p.id));
      onDeleted(p.id);
    } catch (err) {
      setRowError((prev) => ({ ...prev, [p.id]: String(err) }));
    } finally {
      setDeleteBusyId(null);
    }
  }

  return (
    <div style={{ maxWidth: 920, padding: "40px 48px" }}>
      <h1 style={{ fontSize: 32, marginBottom: 6 }}>Home</h1>
      <p className="text-muted" style={{ margin: "0 0 28px", maxWidth: "56ch" }}>
        Open a project root to discover services, or jump back into one you had running.
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
                <td style={{ fontWeight: 600 }}>
                  {renamingId === p.id ? (
                    <div>
                      <input
                        autoFocus
                        style={{ width: "100%", padding: "4px 6px", fontSize: 13, borderRadius: "var(--radius-sm)", border: "1px solid var(--color-divider)", background: "var(--color-bg)" }}
                        value={renameValue}
                        onChange={(e) => setRenameValue(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") void saveRename(p.id);
                          if (e.key === "Escape") cancelRename();
                        }}
                        onBlur={() => {
                          // Blur with an empty name means the user moved on, not that they
                          // rejected a submission - cancel silently instead of showing an
                          // error the user can no longer see (focus has already left).
                          if (!renameValue.trim()) cancelRename();
                          else void saveRename(p.id);
                        }}
                      />
                      {renameError && (
                        <div style={{ color: "var(--color-status-bad-fg)", fontSize: 11, marginTop: 2 }}>
                          {renameError}
                        </div>
                      )}
                    </div>
                  ) : (
                    <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                      {p.name}
                      <IconButton title="Rename" onClick={() => startRename(p)}>
                        <path d="M12 20h9" />
                        <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
                      </IconButton>
                    </span>
                  )}
                </td>
                <td className="mono text-muted" style={{ fontSize: 12 }}>
                  {p.rootPath}
                  {rowError[p.id] && (
                    <div style={{ color: "var(--color-status-bad-fg)", fontSize: 11, marginTop: 4 }}>
                      {rowError[p.id]}
                    </div>
                  )}
                </td>
                <td style={{ textAlign: "right" }}>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                    <IconButton
                      title="Change project folder"
                      onClick={() => editPath(p)}
                      disabled={pathBusyId === p.id || deleteBusyId === p.id}
                    >
                      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
                    </IconButton>
                    <IconButton
                      title="Delete project"
                      color="var(--color-status-bad-fg)"
                      onClick={() => deleteProjectRow(p)}
                      disabled={pathBusyId === p.id || deleteBusyId === p.id}
                    >
                      <path d="M3 6h18" />
                      <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
                      <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
                    </IconButton>
                    <button type="button" className="btn btn-secondary" onClick={() => open(p.rootPath)} disabled={busy}>
                      Open
                    </button>
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
