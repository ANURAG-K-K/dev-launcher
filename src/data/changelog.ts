export interface ChangelogEntry {
  date: string;
  changes: string[];
}

// Source of truth for both the in-app Changelog dialog and CHANGELOG.md.
// Newest first.
export const CHANGELOG: ChangelogEntry[] = [
  {
    date: "2026-08-03",
    changes: ["Inline project rename", "Open repository in VS Code"],
  },
  {
    date: "2026-08-02",
    changes: ["Switch git branch with stash support"],
  },
  {
    date: "2026-08-01",
    changes: ["Configurable terminal shell choice", "Configurable action-button visibility"],
  },
  {
    date: "2026-07-31",
    changes: [
      "Per-repo visible console toggle",
      "Custom app icon",
      "Fixed git status console flashing and UI freeze",
      "Theme apply fix",
    ],
  },
  {
    date: "2026-07-19",
    changes: [
      "Per-repo git branch status (dirty, ahead/behind)",
      "Git pull / fetch actions",
      "Favorite / pin repositories to top",
    ],
  },
  {
    date: "2026-07-18",
    changes: [
      "Initial release: Tauri v2 + React/Vite/TS scaffold",
      "Repository discovery and scanning",
      "Process manager with live status tracking",
      "Per-repo command and env overrides",
      "Sequential launcher with dependency ordering",
      "Named launch profiles",
      "Soft-delete repositories, preserving history",
      "Persisted settings with startup restore",
      "Desktop crash notifications",
      "Launch history view",
      "Log viewer with search, filter, and auto-scroll",
    ],
  },
];
