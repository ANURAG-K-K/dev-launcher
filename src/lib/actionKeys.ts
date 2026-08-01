/**
 * The 9 per-repo action buttons a user can show/hide via Settings (Start/Stop/Restart are
 * always shown by process status, not part of this list). Order here is the order checkboxes
 * appear in Settings; Project.tsx imports the same `key` values to gate its buttons.
 */
export const ACTION_KEYS: { key: string; label: string }[] = [
  { key: "favorite", label: "Star" },
  { key: "visibleConsole", label: "Visible console toggle" },
  { key: "fetch", label: "Fetch" },
  { key: "pull", label: "Pull" },
  { key: "logs", label: "Logs" },
  { key: "openFolder", label: "Open Folder" },
  { key: "openTerminal", label: "Open Terminal" },
  { key: "edit", label: "Edit config" },
  { key: "remove", label: "Remove" },
];
