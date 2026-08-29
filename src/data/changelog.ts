export interface ChangelogEntry {
  date: string;
  changes: string[];
}

// Source of truth for both the in-app Changelog dialog and CHANGELOG.md.
// Newest first.
export const CHANGELOG: ChangelogEntry[] = [
  {
    date: "2026-08-29",
    changes: [
      "Delete a project (with a guard against deleting while any of its repos are running)",
      "Edit a project's root path — existing repos remap in place, keeping their id and overrides",
      "Edit a single repository's path independently",
      "Nested repository detection one level under a non-repo parent folder",
      "Manually add a repository at any path via a folder picker",
      "Split Refresh (lightweight, respects removed repos) from Re-scan (full rediscovery)",
      "Detect a missing project/repository folder and offer Locate… to repoint it",
      "Feedback and GitHub links in the sidebar",
      "Fixed \"Execute All\" mislabeling (renamed to \"Execute\", then \"Launch Project\"/\"Launch\")",
      "Fixed log panel and its dropdowns rendering white in dark mode",
      "Action-button icons shown in Settings, sidebar app icon/name enlarged",
      "Renamed \"Repository\" to \"Service\" throughout the UI",
      "Launch button now reads \"Launch Project\" only when every service is enabled, \"Launch\" otherwise",
      "Documented and verified monorepo compatibility: multiple services can share one git repository",
    ],
  },
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
