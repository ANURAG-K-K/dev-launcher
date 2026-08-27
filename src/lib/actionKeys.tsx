import type { ReactNode } from "react";

/**
 * The 10 per-repo action buttons a user can show/hide via Settings (Start/Stop/Restart are
 * always shown by process status, not part of this list). Order here is the order checkboxes
 * appear in Settings; Project.tsx imports the same `key`/`icon` values to render its buttons.
 */
export const ACTION_KEYS: { key: string; label: string; icon: ReactNode }[] = [
  {
    key: "favorite",
    label: "Star",
    icon: <path d="M12 2l2.9 6.3 6.9.7-5.1 4.6 1.4 6.8L12 17.8 5.9 20.4l1.4-6.8L2.2 9l6.9-.7L12 2z" />,
  },
  {
    key: "visibleConsole",
    label: "Visible console toggle",
    icon: (
      <>
        <rect x="3" y="4" width="18" height="13" rx="1" />
        <path d="M8 21h8 M12 17v4" />
      </>
    ),
  },
  {
    key: "fetch",
    label: "Fetch",
    icon: (
      <>
        <path d="M20 16.58A5 5 0 0 0 18 7h-1.26A8 8 0 1 0 4 15.25" />
        <path d="M12 12v8" />
        <path d="M9 17l3 3 3-3" />
      </>
    ),
  },
  {
    key: "pull",
    label: "Pull",
    icon: (
      <>
        <path d="M12 3v12" />
        <path d="M7 10l5 5 5-5" />
        <path d="M4 21h16" />
      </>
    ),
  },
  { key: "logs", label: "Logs", icon: <path d="M4 6h16M4 12h16M4 18h10" /> },
  {
    key: "openFolder",
    label: "Open Folder",
    icon: <path d="M3 7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />,
  },
  {
    key: "openTerminal",
    label: "Open Terminal",
    icon: (
      <>
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <polyline points="7 9 10 12 7 15" />
        <line x1="12" y1="15" x2="16" y2="15" />
      </>
    ),
  },
  {
    key: "edit",
    label: "Edit config",
    icon: (
      <>
        <path d="M12 20h9" />
        <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
      </>
    ),
  },
  {
    key: "remove",
    label: "Remove",
    icon: (
      <>
        <path d="M3 6h18" />
        <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
        <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
      </>
    ),
  },
  {
    key: "openWithCode",
    label: "Open with VS Code",
    icon: (
      <>
        <polyline points="16 18 22 12 16 6" />
        <polyline points="8 6 2 12 8 18" />
      </>
    ),
  },
];
