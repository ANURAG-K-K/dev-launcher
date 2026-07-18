import { useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { Home } from "@/views/Home";
import { Project } from "@/views/Project";
import { Logs } from "@/views/Logs";
import { Settings } from "@/views/Settings";
import { History } from "@/views/History";
import type { DependencyEdge, Profile, Project as ProjectT, ProjectWithRepos, RepoStatus, Repository, Settings as SettingsT } from "@/types";
import { cn } from "@/lib/utils";
import { applyProfile, getSettings, listDependencies, listProfiles, listRecentProjects, openProject, restartRepo } from "@/api";

type View = "home" | "project" | "logs" | "settings" | "history";

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
  const crashCountsRef = useRef<Record<number, number>>({});

  // Live mirror of repositories for use inside the []-deps event listener (avoids a stale closure).
  const reposRef = useRef<Repository[]>([]);
  useEffect(() => {
    reposRef.current = repositories;
  }, [repositories]);

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

  async function refreshRecent() {
    try {
      setRecentProjects(await listRecentProjects());
    } catch {
      /* ignore */
    }
  }

  async function openRecent(rootPath: string) {
    try {
      applyResult(await openProject(rootPath));
    } catch {
      /* folder gone / unreadable — ignore */
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
      document.documentElement.dataset.theme = settings.theme;

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
            <div style={{ width: 28, height: 28, background: "var(--color-accent-600)", flex: "none" }} />
            <div style={{ fontFamily: "var(--font-heading)", fontSize: 15, fontWeight: 800, letterSpacing: "0.01em", lineHeight: 1.15 }}>
              Dev
              <br />
              Launcher
            </div>
          </div>
        </div>

        <nav style={{ paddingBottom: 8 }}>
          {NAV.map((n) => {
            const disabled = (n.id === "project" || n.id === "history") && !project;
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
                {n.label}
              </button>
            );
          })}
        </nav>

        <div style={{ flex: 1, overflow: "auto", borderTop: "2px solid var(--color-divider)", paddingBottom: 8 }}>
          <div className="sectiontitle" style={{ padding: "12px 16px 6px" }}>Recent</div>
          {recentProjects.length === 0 ? (
            <div className="text-muted" style={{ padding: "0 16px 8px", fontSize: 12 }}>No recent projects</div>
          ) : (
            recentProjects.map((p) => (
              <button
                key={p.id}
                className={cn("navitem", project?.id === p.id && "navitem-active")}
                title={p.rootPath}
                onClick={() => openRecent(p.rootPath)}
                style={{ display: "block", width: "100%", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
              >
                {p.name}
              </button>
            ))
          )}
        </div>

        <div style={{ borderTop: "2px solid var(--color-divider)", padding: "14px 16px", fontSize: 11, opacity: 0.5 }}>
          v0.1.0 · Tauri
        </div>
      </aside>

      <main style={{ flex: 1, minWidth: 0 }}>
        {view === "home" && <Home onOpened={applyResult} />}
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
