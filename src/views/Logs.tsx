/** Logs view (F11): per-repo live log viewer subscribed to repo_log events. */
export function Logs() {
  return (
    <div style={{ padding: "40px 48px", display: "flex", flexDirection: "column", height: "100vh", boxSizing: "border-box" }}>
      <h1 style={{ fontSize: 32, marginBottom: 6 }}>Logs</h1>
      <p className="text-muted" style={{ margin: "0 0 20px" }}>Live process output, one stream per repository</p>

      <div
        style={{
          flex: 1,
          background: "var(--color-neutral-900)",
          color: "var(--color-neutral-100)",
          padding: "18px 20px",
          overflow: "auto",
        }}
      >
        <p className="mono" style={{ fontSize: 12.5, opacity: 0.5, margin: 0 }}>
          Per-repository live log streaming will live here (F11). Scaffold placeholder.
        </p>
      </div>
    </div>
  );
}
