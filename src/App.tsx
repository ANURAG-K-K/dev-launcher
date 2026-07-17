import { useState } from "react";
import { Home } from "@/views/Home";
import { Project } from "@/views/Project";
import { Logs } from "@/views/Logs";
import { Settings } from "@/views/Settings";

type View = "home" | "project" | "logs" | "settings";

const NAV: { id: View; label: string }[] = [
  { id: "home", label: "Home" },
  { id: "project", label: "Project" },
  { id: "logs", label: "Logs" },
  { id: "settings", label: "Settings" },
];

function App() {
  const [view, setView] = useState<View>("home");

  return (
    <div className="flex h-screen text-sm text-neutral-800 dark:text-neutral-200">
      <aside className="w-56 shrink-0 border-r border-neutral-200 bg-neutral-100 p-3 dark:border-neutral-800 dark:bg-neutral-900">
        <div className="mb-4 px-2 font-semibold">Multi-Repo Launcher</div>
        <nav className="flex flex-col gap-1">
          {NAV.map((n) => (
            <button key={n.id} onClick={() => setView(n.id)} className={navClass(view === n.id)}>
              {n.label}
            </button>
          ))}
        </nav>
      </aside>
      <main className="flex-1 overflow-auto p-6">
        {view === "home" && <Home />}
        {view === "project" && <Project />}
        {view === "logs" && <Logs />}
        {view === "settings" && <Settings />}
      </main>
    </div>
  );
}

function navClass(active: boolean) {
  return `rounded-md px-3 py-2 text-left transition-colors ${
    active
      ? "bg-neutral-800 text-white dark:bg-neutral-200 dark:text-neutral-900"
      : "hover:bg-neutral-200 dark:hover:bg-neutral-800"
  }`;
}

export default App;
