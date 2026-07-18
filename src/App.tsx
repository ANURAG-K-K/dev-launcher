import { useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { Home } from "@/views/Home";
import { Project } from "@/views/Project";
import { Logs } from "@/views/Logs";
import { Settings } from "@/views/Settings";
import type { Project as ProjectT, ProjectWithRepos, RepoStatus, Repository } from "@/types";
import { cn } from "@/lib/utils";

type View = "home" | "project" | "logs" | "settings";

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

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<RepoStatus>("repo_status_changed", (event) => {
      const payload = event.payload;
      setStatuses((prev) => ({ ...prev, [payload.repositoryId]: payload }));
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
  }

  function openLogs(repositoryId: number) {
    setSelectedRepoId(repositoryId);
    setView("logs");
  }

  function onRepoUpdated(repo: Repository) {
    setRepositories((prev) => prev.map((r) => (r.id === repo.id ? repo : r)));
  }

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

        <nav style={{ flex: 1, overflow: "auto", paddingBottom: 16 }}>
          {NAV.map((n) => {
            const disabled = n.id === "project" && !project;
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
            onScanned={applyResult}
            onOpenLogs={openLogs}
            onRepoUpdated={onRepoUpdated}
          />
        )}
        {view === "logs" && (
          <Logs repositories={repositories} selectedRepoId={selectedRepoId} onSelectRepo={setSelectedRepoId} />
        )}
        {view === "settings" && <Settings />}
      </main>
    </div>
  );
}

export default App;
