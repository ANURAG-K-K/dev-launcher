import { useEffect, useState } from "react";
import {
  applyProfile,
  createProfile,
  deleteProfile,
  executeProject,
  gitFetch,
  gitPull,
  gitStatusProject,
  gitSwitchBranch,
  openRepoFolder,
  openRepoTerminal,
  openRepoVscode,
  restartRepo,
  scanRepositories,
  setRepositoryEnabled,
  startRepo,
  stopAll,
  stopRepo,
  updateProfile,
  removeRepository,
  setRepositoryFavorite,
  setRepositoryVisibleConsole,
  getSettings,
} from "@/api";
import { confirm } from "@tauri-apps/plugin-dialog";
import type { DependencyEdge, ExecuteResult, Profile, Project as ProjectT, ProjectWithRepos, RepoGitStatus, RepoStatus, Repository } from "@/types";
import { RepoEditDialog } from "./RepoEditDialog";
import { BranchSwitchDialog } from "./BranchSwitchDialog";
import { IconButton } from "@/components/IconButton";

/** Project view (F4): repository list for the open project + Refresh (F2/D3). */
export function Project({
  project,
  repositories,
  statuses,
  initialLaunchDelayMs,
  dependencies,
  profiles,
  activeProfileId,
  setActiveProfileId,
  onScanned,
  onOpenLogs,
  onRepoUpdated,
  onDependenciesChanged,
  onRepositoriesReplaced,
  onProfilesChanged,
}: {
  project: ProjectT | null;
  repositories: Repository[];
  statuses: Record<number, RepoStatus>;
  initialLaunchDelayMs: number;
  dependencies: DependencyEdge[];
  profiles: Profile[];
  activeProfileId: number | null;
  setActiveProfileId: (id: number | null) => void;
  onScanned: (result: ProjectWithRepos) => void;
  onOpenLogs: (repositoryId: number) => void;
  onRepoUpdated: (repo: Repository) => void;
  onDependenciesChanged: () => void;
  onRepositoriesReplaced: (repos: Repository[]) => void;
  onProfilesChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [rowBusy, setRowBusy] = useState<number | null>(null);
  const [rowError, setRowError] = useState<Record<number, string>>({});
  const [enabledBusy, setEnabledBusy] = useState<number | null>(null);
  const [editingRepo, setEditingRepo] = useState<Repository | null>(null);
  const [switchingBranchRepo, setSwitchingBranchRepo] = useState<Repository | null>(null);
  const [executeBusy, setExecuteBusy] = useState(false);
  const [launchDelayMs, setLaunchDelayMs] = useState(initialLaunchDelayMs);
  const [executeResult, setExecuteResult] = useState<ExecuteResult | null>(null);
  const [executeError, setExecuteError] = useState("");
  const [stopBusy, setStopBusy] = useState(false);
  const [profileBusy, setProfileBusy] = useState(false);
  const [profileError, setProfileError] = useState("");
  const [savingProfile, setSavingProfile] = useState(false);
  const [newProfileName, setNewProfileName] = useState("");
  const [gitByRepo, setGitByRepo] = useState<Record<number, RepoGitStatus>>({});

  // Fetched fresh on mount (not passed down from App.tsx) so a change made in Settings takes
  // effect the next time this view is opened, without needing an app restart.
  const [visibleActions, setVisibleActions] = useState<string[]>([
    "logs",
    "openFolder",
    "openTerminal",
    "edit",
    "remove",
  ]);

  useEffect(() => {
    getSettings()
      .then((s) => setVisibleActions(s.visibleActions))
      .catch(() => {
        // Keep the curated-default fallback already in state; nothing to surface here.
      });
  }, []);

  // The execute summary is a one-shot outcome, not live state — auto-dismiss it so it can't go
  // stale as repos are started/stopped individually (the live "running" count below is the truth).
  useEffect(() => {
    if (!executeResult) return;
    const t = setTimeout(() => setExecuteResult(null), 5000);
    return () => clearTimeout(t);
  }, [executeResult]);

  async function loadGitStatus(projectId: number) {
    const list = await gitStatusProject(projectId);
    setGitByRepo(Object.fromEntries(list.map((g) => [g.repositoryId, g])));
  }

  useEffect(() => {
    if (project) void loadGitStatus(project.id);
  }, [project?.id]);

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
      await loadGitStatus(project!.id);
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

  async function executeAll() {
    setExecuteBusy(true);
    setExecuteResult(null);
    setExecuteError("");
    try {
      const result = await executeProject(project!.id, launchDelayMs);
      setExecuteResult(result);
    } catch (err) {
      setExecuteError(String(err));
    } finally {
      setExecuteBusy(false);
    }
  }

  async function stopAllRepos() {
    setStopBusy(true);
    try {
      await stopAll(project!.id);
    } finally {
      setStopBusy(false);
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

  async function removeRepo(repo: Repository) {
    const ok = await confirm(
      `Remove "${repo.name}"? It's stopped and hidden from the list; its launch history is kept, and re-scanning restores it.`,
      { title: "Remove repository", kind: "warning" },
    );
    if (!ok) return;
    await runAction(repo.id, async () => {
      onRepositoriesReplaced(await removeRepository(repo.id));
    });
  }

  const activeProfile = profiles.find((p) => p.id === activeProfileId) ?? null;
  const enabledIds = repositories.filter((r) => r.enabled === 1).map((r) => r.id);

  async function selectProfile(idStr: string) {
    setProfileError("");
    if (!idStr) {
      setActiveProfileId(null);
      return;
    }
    const id = Number(idStr);
    const profile = profiles.find((p) => p.id === id);
    setProfileBusy(true);
    try {
      const updatedRepos = await applyProfile(id);
      onRepositoriesReplaced(updatedRepos);
      if (profile) setLaunchDelayMs(profile.launchDelayMs);
      setActiveProfileId(id);
    } catch (err) {
      setProfileError(String(err));
    } finally {
      setProfileBusy(false);
    }
  }

  async function saveNewProfile() {
    const name = newProfileName.trim();
    if (!name) return;
    setProfileBusy(true);
    setProfileError("");
    try {
      const created = await createProfile(project!.id, name, launchDelayMs, enabledIds);
      onProfilesChanged();
      setActiveProfileId(created.id);
      setNewProfileName("");
      setSavingProfile(false);
    } catch (err) {
      setProfileError(String(err));
    } finally {
      setProfileBusy(false);
    }
  }

  async function updateActiveProfile() {
    if (!activeProfile) return;
    setProfileBusy(true);
    setProfileError("");
    try {
      await updateProfile(activeProfile.id, activeProfile.name, launchDelayMs, enabledIds);
      onProfilesChanged();
    } catch (err) {
      setProfileError(String(err));
    } finally {
      setProfileBusy(false);
    }
  }

  async function deleteActiveProfile() {
    if (!activeProfile) return;
    setProfileBusy(true);
    setProfileError("");
    try {
      await deleteProfile(activeProfile.id);
      onProfilesChanged();
      setActiveProfileId(null);
    } catch (err) {
      setProfileError(String(err));
    } finally {
      setProfileBusy(false);
    }
  }

  const runningCount = repositories.filter((r) => {
    const s = statuses[r.id]?.status;
    return s === "running" || s === "starting";
  }).length;

  // Favorites float to the top; the backend already orders this way, but sort locally so a star
  // toggle re-orders immediately without a re-fetch.
  const sortedRepos = [...repositories].sort(
    (a, b) => b.favorite - a.favorite || a.name.localeCompare(b.name),
  );

  async function toggleFavorite(repo: Repository) {
    try {
      onRepoUpdated(await setRepositoryFavorite(repo.id, repo.favorite !== 1));
    } catch (err) {
      setRowError((prev) => ({ ...prev, [repo.id]: String(err) }));
    }
  }

  async function toggleVisibleConsole(repo: Repository) {
    try {
      onRepoUpdated(await setRepositoryVisibleConsole(repo.id, repo.visibleConsole !== 1));
    } catch (err) {
      setRowError((prev) => ({ ...prev, [repo.id]: String(err) }));
    }
  }

  return (
    <div style={{ padding: "40px 48px 64px" }}>
      <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 16, flexWrap: "wrap", marginBottom: 6 }}>
        <h1 style={{ fontSize: 32 }}>{project.name}</h1>
        <div className="mono text-muted" style={{ fontSize: 12 }}>{project.rootPath}</div>
      </div>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 16, flexWrap: "wrap", marginBottom: 12 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
          <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }} className="text-muted">
            Profile
            <select
              value={activeProfileId ?? ""}
              disabled={profileBusy}
              onChange={(e) => selectProfile(e.target.value)}
              style={{
                padding: "6px 8px",
                border: "1px solid var(--color-divider)",
                background: "var(--color-bg)",
                borderRadius: "var(--radius-md)",
                color: "var(--color-text)",
              }}
            >
              <option value="">— No profile —</option>
              {profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>

          {!savingProfile && (
            <button
              type="button"
              className="btn btn-ghost"
              onClick={() => setSavingProfile(true)}
              disabled={profileBusy}
            >
              Save as profile
            </button>
          )}
          {savingProfile && (
            <>
              <input
                type="text"
                autoFocus
                placeholder="Profile name"
                value={newProfileName}
                onChange={(e) => setNewProfileName(e.target.value)}
                style={{
                  width: 160,
                  padding: "6px 8px",
                  border: "1px solid var(--color-divider)",
                  background: "var(--color-bg)",
                  borderRadius: "var(--radius-md)",
                  color: "var(--color-text)",
                }}
              />
              <button
                type="button"
                className="btn btn-primary"
                onClick={saveNewProfile}
                disabled={profileBusy || !newProfileName.trim()}
              >
                Save
              </button>
              <button
                type="button"
                className="btn btn-ghost"
                onClick={() => {
                  setSavingProfile(false);
                  setNewProfileName("");
                }}
                disabled={profileBusy}
              >
                Cancel
              </button>
            </>
          )}

          {activeProfile && !savingProfile && (
            <>
              <button type="button" className="btn btn-ghost" onClick={updateActiveProfile} disabled={profileBusy}>
                Update
              </button>
              <button type="button" className="btn btn-ghost" onClick={deleteActiveProfile} disabled={profileBusy}>
                Delete
              </button>
            </>
          )}

          <span className="text-muted" style={{ fontSize: 11 }}>
            Applying a profile sets which repositories are enabled.
          </span>
          {profileError && (
            <span style={{ color: "var(--color-status-bad-fg)", fontSize: 11 }}>{profileError}</span>
          )}
        </div>
      </div>

      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 16, flexWrap: "wrap", marginBottom: 22 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <p className="text-muted" style={{ margin: 0 }}>
            {repositories.length} {repositories.length === 1 ? "repository" : "repositories"} discovered
          </p>
          {runningCount > 0 && <span className="tag tag-good">● {runningCount} running</span>}
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
          <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }} className="text-muted">
            Delay (ms)
            <input
              type="number"
              min={0}
              value={launchDelayMs}
              onChange={(e) => setLaunchDelayMs(Math.max(0, Number(e.target.value)))}
              style={{
                width: 90,
                padding: "6px 8px",
                border: "1px solid var(--color-divider)",
                background: "var(--color-bg)",
                borderRadius: "var(--radius-md)",
              }}
            />
          </label>
          <button type="button" className="btn btn-primary" onClick={executeAll} disabled={executeBusy}>
            {executeBusy ? "Executing…" : "Execute All"}
          </button>
          <button
            type="button"
            className="btn btn-secondary"
            onClick={stopAllRepos}
            disabled={stopBusy || runningCount === 0}
          >
            {stopBusy ? "Stopping…" : "Stop All"}
          </button>
          <button type="button" className="btn btn-secondary" onClick={refresh} disabled={busy}>
            {busy ? "Scanning…" : "Refresh"}
          </button>
        </div>
      </div>

      {(executeResult || executeError) && (
        <div style={{ marginBottom: 16, fontSize: 13 }}>
          {executeResult && (
            <span>
              <span className="tag tag-good">Started {executeResult.started.length}</span>{" "}
              <span className="tag tag-neutral">Skipped {executeResult.skipped.length}</span>
            </span>
          )}
          {executeError && <div style={{ color: "var(--color-status-bad-fg)" }}>{executeError}</div>}
        </div>
      )}

      {repositories.length === 0 ? (
        <p className="text-muted">No repositories with a package.json found under this root.</p>
      ) : (
        <div style={{ overflowX: "auto" }}>
          <table className="table" style={{ minWidth: 640 }}>
            <thead>
              <tr>
                <th>Enabled</th>
                <th>Repository</th>
                <th>Branch</th>
                <th>PID</th>
                <th>Package Manager</th>
                <th>Command</th>
                <th>Status</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {sortedRepos.map((r) => {
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
                    <td style={{ fontSize: 12 }}>
                      {gitByRepo[r.id] ? (
                        <span
                          role="button"
                          title="Click to switch branch"
                          style={{
                            display: "inline-flex",
                            alignItems: "center",
                            gap: 6,
                            cursor: busyRow ? "default" : "pointer",
                            opacity: busyRow ? 0.5 : 1,
                          }}
                          onClick={() => {
                            if (busyRow) return;
                            setSwitchingBranchRepo(r);
                          }}
                        >
                          <span className="mono">
                            {gitByRepo[r.id].branch === "HEAD" ? "Detached HEAD" : gitByRepo[r.id].branch}
                          </span>
                          {gitByRepo[r.id].dirty && (
                            <span title="Uncommitted changes" style={{ color: "var(--color-status-bad-fg)" }}>
                              ●
                            </span>
                          )}
                          {(gitByRepo[r.id].ahead > 0 || gitByRepo[r.id].behind > 0) && (
                            <span className="text-muted mono" style={{ fontSize: 11 }}>
                              {gitByRepo[r.id].ahead > 0 && (
                                <span title="Commits ahead of upstream">↑{gitByRepo[r.id].ahead}</span>
                              )}
                              {gitByRepo[r.id].behind > 0 && (
                                <span title="Commits behind upstream">↓{gitByRepo[r.id].behind}</span>
                              )}
                            </span>
                          )}
                        </span>
                      ) : (
                        <span className="text-muted">—</span>
                      )}
                    </td>
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
                        {visibleActions.includes("favorite") && (
                          <IconButton
                            title={r.favorite === 1 ? "Unstar" : "Star"}
                            color={r.favorite === 1 ? "var(--color-accent)" : undefined}
                            onClick={() => toggleFavorite(r)}
                          >
                            <path
                              fill={r.favorite === 1 ? "currentColor" : "none"}
                              d="M12 2l2.9 6.3 6.9.7-5.1 4.6 1.4 6.8L12 17.8 5.9 20.4l1.4-6.8L2.2 9l6.9-.7L12 2z"
                            />
                          </IconButton>
                        )}
                        {visibleActions.includes("visibleConsole") && (
                          <IconButton
                            title={
                              r.visibleConsole === 1
                                ? "Visible console (on) — launches in a window you can type into; logs aren't captured"
                                : "Visible console (off) — launches in the background with captured logs"
                            }
                            color={r.visibleConsole === 1 ? "var(--color-accent)" : undefined}
                            onClick={() => toggleVisibleConsole(r)}
                          >
                            <rect x="3" y="4" width="18" height="13" rx="1" />
                            <path d="M8 21h8 M12 17v4" />
                          </IconButton>
                        )}
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
                        {gitByRepo[r.id] && visibleActions.includes("fetch") && (
                          <IconButton
                            title="Fetch"
                            disabled={busyRow}
                            onClick={() => runAction(r.id, () => gitFetch(r.id))}
                          >
                            <path d="M20 16.58A5 5 0 0 0 18 7h-1.26A8 8 0 1 0 4 15.25" />
                            <path d="M12 12v8" />
                            <path d="M9 17l3 3 3-3" />
                          </IconButton>
                        )}
                        {gitByRepo[r.id] && visibleActions.includes("pull") && (
                          <IconButton
                            title="Pull"
                            disabled={busyRow}
                            onClick={() =>
                              runAction(r.id, async () => {
                                await gitPull(r.id);
                                await loadGitStatus(project!.id);
                              })
                            }
                          >
                            <path d="M12 3v12" />
                            <path d="M7 10l5 5 5-5" />
                            <path d="M4 21h16" />
                          </IconButton>
                        )}
                        {visibleActions.includes("logs") && (
                          <IconButton title="Logs" onClick={() => onOpenLogs(r.id)}>
                            <path d="M4 6h16M4 12h16M4 18h10" />
                          </IconButton>
                        )}
                        {visibleActions.includes("openFolder") && (
                          <IconButton
                            title="Open Folder"
                            disabled={busyRow}
                            onClick={() => runAction(r.id, () => openRepoFolder(r.id))}
                          >
                            <path d="M3 7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
                          </IconButton>
                        )}
                        {visibleActions.includes("openTerminal") && (
                          <IconButton
                            title="Open Terminal"
                            disabled={busyRow}
                            onClick={() => runAction(r.id, () => openRepoTerminal(r.id))}
                          >
                            <rect x="3" y="4" width="18" height="16" rx="2" />
                            <polyline points="7 9 10 12 7 15" />
                            <line x1="12" y1="15" x2="16" y2="15" />
                          </IconButton>
                        )}
                        {visibleActions.includes("openWithCode") && (
                          <IconButton
                            title="Open with VS Code"
                            disabled={busyRow}
                            onClick={() => runAction(r.id, () => openRepoVscode(r.id))}
                          >
                            <polyline points="16 18 22 12 16 6" />
                            <polyline points="8 6 2 12 8 18" />
                          </IconButton>
                        )}
                        {visibleActions.includes("edit") && (
                          <IconButton title="Edit config" onClick={() => setEditingRepo(r)}>
                            <path d="M12 20h9" />
                            <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
                          </IconButton>
                        )}
                        {visibleActions.includes("remove") && (
                          <IconButton
                            title="Remove"
                            color="var(--color-status-bad-fg)"
                            disabled={busyRow}
                            onClick={() => removeRepo(r)}
                          >
                            <path d="M3 6h18" />
                            <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
                            <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
                          </IconButton>
                        )}
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

      {editingRepo && (
        <RepoEditDialog
          repo={editingRepo}
          repositories={repositories}
          dependencies={dependencies}
          onClose={() => setEditingRepo(null)}
          onRepoUpdated={onRepoUpdated}
          onDependenciesChanged={onDependenciesChanged}
        />
      )}
      {switchingBranchRepo && gitByRepo[switchingBranchRepo.id] && (
        <BranchSwitchDialog
          repo={switchingBranchRepo}
          currentStatus={gitByRepo[switchingBranchRepo.id]}
          switching={rowBusy === switchingBranchRepo.id}
          onClose={() => setSwitchingBranchRepo(null)}
          onSwitch={async (branch, stash, stashMessage, stashUntracked) => {
            setRowBusy(switchingBranchRepo.id);
            try {
              const status = await gitSwitchBranch({
                repositoryId: switchingBranchRepo.id,
                branch,
                stash,
                stashMessage,
                stashUntracked,
              });
              setGitByRepo((prev) => ({ ...prev, [switchingBranchRepo.id]: status }));
            } finally {
              setRowBusy(null);
            }
          }}
        />
      )}
    </div>
  );
}
