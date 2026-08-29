import { useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Home } from "@/views/Home";
import { Project } from "@/views/Project";
import { Logs } from "@/views/Logs";
import { Settings } from "@/views/Settings";
import { History } from "@/views/History";
import { ChangelogDialog } from "@/views/ChangelogDialog";
import type { DependencyEdge, Profile, Project as ProjectT, ProjectWithRepos, RepoStatus, Repository, Settings as SettingsT } from "@/types";
import { cn } from "@/lib/utils";
import { applyProfile, getSettings, listDependencies, listProfiles, listRecentProjects, openProject, pickDirectory, projectRunningCounts, renameProject, restartRepo, updateProjectPath } from "@/api";
import { applyTheme } from "@/lib/theme";
import { IconButton } from "@/components/IconButton";

type View = "home" | "project" | "logs" | "settings" | "history";

const GITHUB_URL = "https://github.com/ANURAG-K-K/dev-launcher";
const FEEDBACK_URL = "https://forms.gle/RLTKySxYLspbf2pj8";

const NAV: { id: View; label: string; icon: ReactNode }[] = [
  {
    id: "home",
    label: "Home",
    icon: (
      <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z M9 22V12h6v10" />
    ),
  },
  {
    id: "project",
    label: "Project",
    icon: (
      <path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12 M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17" />
    ),
  },
  {
    id: "logs",
    label: "Logs",
    icon: (
      <path d="M4 7V4a1 1 0 0 1 1-1h4 M4 17v3a1 1 0 0 0 1 1h4 M15 3h4a1 1 0 0 1 1 1v3 M20 17v3a1 1 0 0 1-1 1h-4 M8 12h8 M8 8h4 M8 16h6" />
    ),
  },
  {
    id: "history",
    label: "History",
    icon: (
      <>
        <circle cx="12" cy="12" r="9" />
        <path d="M12 7v5l3 3" />
      </>
    ),
  },
  {
    id: "settings",
    label: "Settings",
    icon: (
      <>
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
      </>
    ),
  },
];

function App() {
  const [view, setView] = useState<View>("home");
  const [project, setProject] = useState<ProjectT | null>(null);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [statuses, setStatuses] = useState<Record<number, RepoStatus>>({});
  const [selectedRepoId, setSelectedRepoId] = useState<number | null>(null);
  const [dependencies, setDependencies] = useState<DependencyEdge[]>([]);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [activeProfileId, setActiveProfileId] = useState<number | null>(null);
  const [recentProjects, setRecentProjects] = useState<ProjectT[]>([]);
  const [settings, setSettings] = useState<SettingsT | null>(null);
  const [renamingProjectId, setRenamingProjectId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameError, setRenameError] = useState("");
  const [recentOpenError, setRecentOpenError] = useState<Record<number, string>>({});
  const [locatingProjectId, setLocatingProjectId] = useState<number | null>(null);
  const [sidebarFilter, setSidebarFilter] = useState("");
  const [runningCounts, setRunningCounts] = useState<Record<number, number>>({});
  const renameCancelledRef = useRef(false);
  const crashCountsRef = useRef<Record<number, number>>({});
  const [appVersion, setAppVersion] = useState("");
  const [changelogOpen, setChangelogOpen] = useState(false);

  useEffect(() => {
    void getVersion().then(setAppVersion);
  }, []);

  // Live mirror of repositories for use inside the []-deps event listener (avoids a stale closure).
  const reposRef = useRef<Repository[]>([]);
  useEffect(() => {
    reposRef.current = repositories;
  }, [repositories]);

  // Live mirror of recentProjects for the same reason (used inside the []-deps status listener).
  const recentProjectsRef = useRef<ProjectT[]>([]);
  useEffect(() => {
    recentProjectsRef.current = recentProjects;
  }, [recentProjects]);

  /** Per-project count of currently-running services, for the sidebar's running indicator. */
  async function refreshRunningCounts(projectIds: number[]) {
    if (projectIds.length === 0) {
      setRunningCounts({});
      return;
    }
    try {
      setRunningCounts(await projectRunningCounts(projectIds));
    } catch {
      /* ignore — indicator just stays at its last known state */
    }
  }

  /** Notify on crash (F15), gated by the live setting — read fresh so a toggle takes effect at once. */
  async function notifyCrash(repositoryId: number) {
    let settings;
    try {
      settings = await getSettings();
    } catch {
      return;
    }
    if (!settings.notificationsEnabled) return;
    const name = reposRef.current.find((r) => r.id === repositoryId)?.name ?? `#${repositoryId}`;
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title: "Repository crashed", body: `${name} exited unexpectedly.` });
  }

  /** Auto-restart a crashed repo if enabled (F14), capped per session to avoid crash loops. */
  async function maybeAutoRestart(repositoryId: number) {
    let s;
    try {
      s = await getSettings();
    } catch {
      return;
    }
    if (!s.autoRestart) return;
    const counts = crashCountsRef.current;
    const n = counts[repositoryId] ?? 0;
    if (n >= 3) return; // ponytail: flat per-session cap, no backoff
    counts[repositoryId] = n + 1;
    try {
      await restartRepo(repositoryId);
    } catch {
      /* ignore */
    }
  }

  async function refreshDependencies(projectId: number) {
    setDependencies(await listDependencies(projectId));
  }

  async function refreshProfiles(projectId: number) {
    setProfiles(await listProfiles(projectId));
  }

  useEffect(() => {
    setActiveProfileId(null);
    if (project) {
      refreshDependencies(project.id);
      refreshProfiles(project.id);
    } else {
      setDependencies([]);
      setProfiles([]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.id]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<RepoStatus>("repo_status_changed", (event) => {
      const payload = event.payload;
      setStatuses((prev) => ({ ...prev, [payload.repositoryId]: payload }));
      if (payload.status === "crashed") {
        void notifyCrash(payload.repositoryId);
        void maybeAutoRestart(payload.repositoryId);
      }
      void refreshRunningCounts(recentProjectsRef.current.map((p) => p.id));
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  function applyResult(result: ProjectWithRepos) {
    setProject(result.project);
    setRepositories(result.repositories);
    setView("project");
    refreshDependencies(result.project.id);
    refreshRecent();
  }

  function applyPathUpdate(result: ProjectWithRepos) {
    setRecentProjects((prev) => prev.map((p) => (p.id === result.project.id ? result.project : p)));
    if (project?.id === result.project.id) {
      setProject(result.project);
      setRepositories(result.repositories);
    }
  }

  function applyProjectDeleted(projectId: number) {
    setRecentProjects((prev) => prev.filter((p) => p.id !== projectId));
    if (project?.id === projectId) {
      setProject(null);
      setRepositories([]);
      setView("home");
    }
  }

  async function refreshRecent() {
    try {
      const recent = await listRecentProjects();
      setRecentProjects(recent);
      void refreshRunningCounts(recent.map((p) => p.id));
    } catch {
      /* ignore */
    }
  }

  async function openRecent(p: ProjectT) {
    setRecentOpenError((prev) => ({ ...prev, [p.id]: "" }));
    try {
      applyResult(await openProject(p.rootPath));
    } catch (err) {
      setRecentOpenError((prev) => ({ ...prev, [p.id]: String(err) }));
    }
  }

  async function locateProject(p: ProjectT) {
    const dir = await pickDirectory();
    if (!dir) return;
    setLocatingProjectId(p.id);
    try {
      const result = await updateProjectPath(p.id, dir);
      setRecentProjects((prev) => prev.map((x) => (x.id === result.project.id ? result.project : x)));
      setRecentOpenError((prev) => ({ ...prev, [p.id]: "" }));
      applyResult(result);
    } catch (err) {
      setRecentOpenError((prev) => ({ ...prev, [p.id]: String(err) }));
    } finally {
      setLocatingProjectId(null);
    }
  }

  function startRename(p: ProjectT) {
    // Cleared unconditionally on every edit-start (not just consumed-and-reset on the blur
    // path in saveRename): correctness can't depend on the browser firing `blur` when a
    // focused element unmounts — Chromium/WebView2 (this app's actual runtime) doesn't.
    renameCancelledRef.current = false;
    setRenamingProjectId(p.id);
    setRenameValue(p.name);
    setRenameError("");
  }

  function cancelRename() {
    renameCancelledRef.current = true;
    setRenamingProjectId(null);
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
      setRecentProjects((prev) => prev.map((p) => (p.id === id ? updated : p)));
      if (project?.id === id) setProject(updated);
      setRenamingProjectId(null);
      setRenameError("");
    } catch (err) {
      setRenameError(String(err));
    }
  }

  function openLogs(repositoryId: number) {
    setSelectedRepoId(repositoryId);
    setView("logs");
  }

  function onRepoUpdated(repo: Repository) {
    setRepositories((prev) => prev.map((r) => (r.id === repo.id ? repo : r)));
  }

  function onRepositoriesReplaced(repos: Repository[]) {
    setRepositories(repos);
  }

  function onProfilesChanged() {
    if (project) refreshProfiles(project.id);
  }

  useEffect(() => {
    async function restore() {
      let settings;
      try {
        settings = await getSettings();
      } catch {
        return;
      }
      setSettings(settings);
      applyTheme(settings.theme);

      if (!settings.restoreLastProject) return;
      try {
        const recent = await listRecentProjects();
        if (recent.length === 0) return;
        const result = await openProject(recent[0].rootPath);
        applyResult(result);

        if (settings.restoreLastSelection && result.project.lastProfileId != null) {
          const repos = await applyProfile(result.project.lastProfileId);
          onRepositoriesReplaced(repos);
        }
      } catch {
        // Folder deleted, project missing, etc. Fall back to Home.
      }
    }
    restore();
    refreshRecent();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Open at a standard size scaled to the screen (~85%, capped), centered. The user can then
  // freely resize or maximize — no aspect-ratio snapping. Min size is set in tauri.conf.json.
  useEffect(() => {
    const appWindow = getCurrentWindow();
    (async () => {
      try {
        const w = Math.min(Math.round(window.screen.availWidth * 0.85), 1600);
        const h = Math.min(Math.round(window.screen.availHeight * 0.85), 1000);
        await appWindow.setSize(new LogicalSize(w, h));
        await appWindow.center();
      } catch {
        /* ignore — fall back to the configured default size */
      }
    })();
  }, []);

  const filteredRecent = sidebarFilter.trim()
    ? recentProjects.filter((p) => p.name.toLowerCase().includes(sidebarFilter.trim().toLowerCase()))
    : recentProjects;

  return (
    <div style={{ display: "flex", minHeight: "100vh" }}>
      <aside
        style={{
          width: 248,
          flex: "none",
          background: "var(--color-surface)",
          borderRight: "2px solid var(--color-divider)",
          position: "sticky",
          top: 0,
          height: "100vh",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <div style={{ padding: "22px 16px 18px", borderBottom: "2px solid var(--color-divider)" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <img src="/logo.svg" width={38} height={38} alt="" style={{ flex: "none" }} />
            <div style={{ fontFamily: "var(--font-heading)", fontSize: 18, fontWeight: 800, letterSpacing: "0.01em", lineHeight: 1.15 }}>
              Dev
              <br />
              Launcher
            </div>
          </div>
        </div>

        <nav style={{ paddingBottom: 8 }}>
          {NAV.map((n) => {
            const disabled = (n.id === "project" || n.id === "history") && !project;
            const label = n.id === "project" && project ? `Project [${project.name}]` : n.label;
            return (
              <button
                key={n.id}
                onClick={() => setView(n.id)}
                disabled={disabled}
                className={cn("navitem", view === n.id && "navitem-active")}
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  {n.icon}
                </svg>
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{label}</span>
              </button>
            );
          })}
        </nav>

        <div style={{ flex: 1, overflow: "auto", borderTop: "2px solid var(--color-divider)", paddingBottom: 8 }}>
          <div className="sectiontitle" style={{ padding: "12px 16px 6px" }}>Recent</div>
          {recentProjects.length > 0 && (
            <div style={{ padding: "0 16px 8px" }}>
              <input
                type="text"
                placeholder="Filter projects…"
                value={sidebarFilter}
                onChange={(e) => setSidebarFilter(e.target.value)}
                style={{
                  width: "100%",
                  fontSize: 12,
                  padding: "4px 8px",
                  border: "1px solid var(--color-divider)",
                  background: "var(--color-bg)",
                  borderRadius: "var(--radius-sm)",
                  color: "var(--color-text)",
                  boxSizing: "border-box",
                }}
              />
            </div>
          )}
          {recentProjects.length === 0 ? (
            <div className="text-muted" style={{ padding: "0 16px 8px", fontSize: 12 }}>No recent projects</div>
          ) : filteredRecent.length === 0 ? (
            <div className="text-muted" style={{ padding: "0 16px 8px", fontSize: 12 }}>No projects match "{sidebarFilter}"</div>
          ) : (
            filteredRecent.map((p) =>
              renamingProjectId === p.id ? (
                <div key={p.id} style={{ padding: "4px 16px" }}>
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
                      // rejected a submission — cancel silently instead of showing an
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
                <div key={p.id}>
                  <div style={{ display: "flex", alignItems: "center", paddingRight: 4 }}>
                    <button
                      className={cn("navitem", project?.id === p.id && "navitem-active")}
                      title={runningCounts[p.id] > 0 ? `${p.rootPath} — ${runningCounts[p.id]} running` : p.rootPath}
                      onClick={() => openRecent(p)}
                      style={{ flex: 1, minWidth: 0, display: "block", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", textAlign: "left" }}
                    >
                      {runningCounts[p.id] > 0 && (
                        <span style={{ color: "var(--color-status-good-fg)", marginRight: 5 }}>●</span>
                      )}
                      {p.name}
                    </button>
                    <IconButton title="Rename" onClick={() => startRename(p)}>
                      <path d="M12 20h9" />
                      <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
                    </IconButton>
                  </div>
                  {recentOpenError[p.id] && (
                    <div
                      style={{
                        margin: "2px 16px 6px",
                        padding: "6px 8px",
                        borderRadius: "var(--radius-sm)",
                        background: "var(--color-status-bad-bg)",
                        color: "var(--color-status-bad-fg)",
                        fontSize: 11,
                        display: "flex",
                        flexDirection: "column",
                        gap: 6,
                      }}
                    >
                      <span>⚠ Folder not found — the project may have been moved or deleted.</span>
                      <button
                        type="button"
                        className="btn btn-secondary"
                        style={{ alignSelf: "flex-start", padding: "3px 8px", fontSize: 11 }}
                        onClick={() => locateProject(p)}
                        disabled={locatingProjectId === p.id}
                      >
                        {locatingProjectId === p.id ? "Locating…" : "Locate…"}
                      </button>
                    </div>
                  )}
                </div>
              )
            )
          )}
        </div>

        <div style={{ display: "flex", alignItems: "center", borderTop: "2px solid var(--color-divider)" }}>
          <button
            type="button"
            onClick={() => setChangelogOpen(true)}
            title="View changelog"
            style={{
              flex: 1,
              padding: "14px 16px",
              fontSize: 11,
              opacity: 0.5,
              background: "none",
              textAlign: "left",
              cursor: "pointer",
              color: "inherit",
            }}
          >
            {appVersion ? `v${appVersion}` : ""}
          </button>
          <button
            type="button"
            onClick={() => void openUrl(FEEDBACK_URL)}
            title="Report a bug, request a feature, or leave feedback"
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 5,
              padding: "8px 10px",
              fontSize: 11,
              fontWeight: 700,
              background: "none",
              border: "none",
              cursor: "pointer",
              color: "inherit",
            }}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
            </svg>
            Feedback
          </button>
          <IconButton title="View on GitHub" onClick={() => void openUrl(GITHUB_URL)}>
            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
            <polyline points="15 3 21 3 21 9" />
            <line x1="10" y1="14" x2="21" y2="3" />
          </IconButton>
        </div>
      </aside>

      {changelogOpen && (
        <ChangelogDialog version={appVersion} onClose={() => setChangelogOpen(false)} />
      )}

      <main style={{ flex: 1, minWidth: 0 }}>
        {view === "home" && (
          <Home
            onOpened={applyResult}
            onRenamed={(u) => {
              setRecentProjects((prev) => prev.map((p) => (p.id === u.id ? u : p)));
              if (project?.id === u.id) setProject(u);
            }}
            onPathUpdated={applyPathUpdate}
            onDeleted={applyProjectDeleted}
          />
        )}
        {view === "project" && (
          <Project
            project={project}
            repositories={repositories}
            statuses={statuses}
            initialLaunchDelayMs={settings?.launchDelayMs ?? 1000}
            dependencies={dependencies}
            profiles={profiles}
            activeProfileId={activeProfileId}
            setActiveProfileId={setActiveProfileId}
            onScanned={applyResult}
            onOpenLogs={openLogs}
            onRepoUpdated={onRepoUpdated}
            onDependenciesChanged={() => project && refreshDependencies(project.id)}
            onRepositoriesReplaced={onRepositoriesReplaced}
            onProfilesChanged={onProfilesChanged}
          />
        )}
        {view === "logs" && (
          <Logs repositories={repositories} selectedRepoId={selectedRepoId} onSelectRepo={setSelectedRepoId} />
        )}
        {view === "history" && <History project={project} />}
        {view === "settings" && <Settings />}
      </main>
    </div>
  );
}

export default App;
