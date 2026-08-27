import { useEffect, useState } from "react";
import { getSettings, updateSettings } from "@/api";
import { applyTheme } from "@/lib/theme";
import { ACTION_KEYS } from "@/lib/actionKeys";
import { IconButton } from "@/components/IconButton";
import { GuideDialog } from "@/views/GuideDialog";
import type { Settings as SettingsT } from "@/types";

/** Settings view (F14): theme, launch delay, auto-detect, restore, auto-restart, etc. */
export function Settings() {
  const [settings, setSettings] = useState<SettingsT | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [guideOpen, setGuideOpen] = useState(false);

  useEffect(() => {
    getSettings().then(setSettings).catch((e) => setError(String(e)));
  }, []);

  async function save(next: SettingsT) {
    const prev = settings;
    setSettings(next);
    setError(null);
    try {
      const result = await updateSettings(next);
      setSettings(result);
      applyTheme(result.theme);
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (e) {
      setSettings(prev);
      setError(String(e));
    }
  }

  if (!settings) {
    return (
      <div style={{ maxWidth: 680, padding: "40px 48px 64px" }}>
        <h1 style={{ fontSize: 32, marginBottom: 6 }}>Settings</h1>
        <p className="text-muted">Loading…</p>
      </div>
    );
  }

  function set<K extends keyof SettingsT>(key: K, value: SettingsT[K]) {
    if (!settings) return;
    save({ ...settings, [key]: value });
  }

  return (
    <div style={{ maxWidth: 680, padding: "40px 48px 64px" }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
        <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
          <h1 style={{ fontSize: 32, marginBottom: 6 }}>Settings</h1>
          <IconButton title="Guide: how to use this app" onClick={() => setGuideOpen(true)}>
            <circle cx="12" cy="12" r="10" />
            <line x1="12" y1="16" x2="12" y2="12" />
            <line x1="12" y1="8" x2="12.01" y2="8" />
          </IconButton>
        </div>
        {saved && <span className="text-muted" style={{ fontSize: 12 }}>Saved</span>}
      </div>
      {guideOpen && <GuideDialog onClose={() => setGuideOpen(false)} />}
      <p className="text-muted" style={{ margin: "0 0 28px" }}>
        Preferences are stored locally and applied on next launch.
      </p>

      {error && (
        <p style={{ color: "var(--color-status-bad-fg)", marginBottom: 18 }}>{error}</p>
      )}

      <div className="sectiontitle" style={{ padding: "0 0 8px" }}>Appearance</div>
      <div className="hr" style={{ margin: "0 0 18px" }} />
      <div style={{ marginBottom: 6 }}>
        <label style={{ display: "block", marginBottom: 6 }}>Theme</label>
        <select
          value={settings.theme}
          onChange={(e) => set("theme", e.target.value as SettingsT["theme"])}
        >
          <option value="light">Light</option>
          <option value="dark">Dark</option>
          <option value="system">System</option>
        </select>
      </div>
      <p className="text-muted" style={{ margin: "6px 0 28px" }}>
        Applies immediately. "System" follows your OS light/dark setting.
      </p>

      <div className="sectiontitle" style={{ padding: "0 0 8px" }}>Launch</div>
      <div className="hr" style={{ margin: "0 0 18px" }} />
      <div style={{ marginBottom: 14 }}>
        <label style={{ display: "block", marginBottom: 6 }}>Launch delay (ms)</label>
        <input
          type="number"
          min={0}
          value={settings.launchDelayMs}
          onChange={(e) => set("launchDelayMs", Math.max(0, Number(e.target.value)))}
        />
      </div>
      <label style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
        <input
          type="checkbox"
          checked={settings.autoDetect}
          onChange={(e) => set("autoDetect", e.target.checked)}
        />
        Auto-detect repositories on open
      </label>
      <label style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 28 }}>
        <input
          type="checkbox"
          checked={settings.autoRestart}
          onChange={(e) => set("autoRestart", e.target.checked)}
        />
        Auto-restart crashed processes
      </label>

      <div className="sectiontitle" style={{ padding: "0 0 8px" }}>Startup</div>
      <div className="hr" style={{ margin: "0 0 18px" }} />
      <label style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
        <input
          type="checkbox"
          checked={settings.restoreLastProject}
          onChange={(e) => set("restoreLastProject", e.target.checked)}
        />
        Reopen the last project on launch
      </label>
      <label style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 28 }}>
        <input
          type="checkbox"
          checked={settings.restoreLastSelection}
          onChange={(e) => set("restoreLastSelection", e.target.checked)}
        />
        Restore the last-used profile selection
      </label>

      <div className="sectiontitle" style={{ padding: "0 0 8px" }}>Other</div>
      <div className="hr" style={{ margin: "0 0 18px" }} />
      <div style={{ marginBottom: 28 }}>
        <label style={{ display: "block", marginBottom: 6 }}>Log history (days)</label>
        <input
          type="number"
          min={0}
          value={settings.logRetention}
          onChange={(e) => set("logRetention", Math.max(0, Number(e.target.value)))}
        />
      </div>
      <div style={{ marginBottom: 6 }}>
        <label style={{ display: "block", marginBottom: 6 }}>Terminal shell</label>
        <select
          value={settings.terminalShell}
          onChange={(e) => set("terminalShell", e.target.value as SettingsT["terminalShell"])}
        >
          <option value="cmd">cmd</option>
          <option value="powershell">PowerShell</option>
        </select>
        <p className="text-muted" style={{ margin: "6px 0 28px" }}>
          Used for "Open Terminal" and for running every repository's dev-server command. Command
          overrides must use PowerShell syntax when PowerShell is selected (e.g. ";" instead of
          "&&", "$env:VAR" instead of "%VAR%").
        </p>
      </div>

      <div className="sectiontitle" style={{ padding: "0 0 8px" }}>Notifications</div>
      <div className="hr" style={{ margin: "0 0 18px" }} />
      <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <input
          type="checkbox"
          checked={settings.notificationsEnabled}
          onChange={(e) => set("notificationsEnabled", e.target.checked)}
        />
        Enable desktop notifications
      </label>
      <p className="text-muted" style={{ margin: "6px 0 0" }}>
        Sends a desktop notification when a repository crashes.
      </p>

      <div className="sectiontitle" style={{ padding: "24px 0 8px" }}>Action buttons</div>
      <div className="hr" style={{ margin: "0 0 18px" }} />
      <p className="text-muted" style={{ margin: "0 0 14px" }}>
        Choose which action buttons appear on each repository row.
      </p>
      {ACTION_KEYS.map(({ key, label, icon }) => (
        <label key={key} style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
          <input
            type="checkbox"
            checked={settings.visibleActions.includes(key)}
            onChange={(e) =>
              set(
                "visibleActions",
                e.target.checked
                  ? [...settings.visibleActions, key]
                  : settings.visibleActions.filter((k) => k !== key)
              )
            }
          />
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{ flex: "none" }}
          >
            {icon}
          </svg>
          {label}
        </label>
      ))}
    </div>
  );
}
