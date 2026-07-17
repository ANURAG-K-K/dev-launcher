/** Settings view (F14): theme, launch delay, auto-detect, restore, auto-restart, etc. */
export function Settings() {
  return (
    <div style={{ maxWidth: 680, padding: "40px 48px 64px" }}>
      <h1 style={{ fontSize: 32, marginBottom: 6 }}>Settings</h1>
      <p className="text-muted" style={{ margin: "0 0 28px" }}>
        Preferences are stored locally and applied on next launch.
      </p>

      <div className="sectiontitle" style={{ padding: "0 0 8px" }}>General</div>
      <div className="hr" style={{ margin: "0 0 18px" }} />
      <p className="text-muted">Application preferences will live here (F14). Scaffold placeholder.</p>
    </div>
  );
}
