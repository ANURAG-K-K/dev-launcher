const SECTIONS: { title: string; body: string }[] = [
  {
    title: "1. Open a project",
    body: "Pick a project root folder on Home. Its immediate child folders containing a package.json are auto-discovered as repositories.",
  },
  {
    title: "2. Configure repositories",
    body: "Select a repository to override its dev command, env file, or dependency order. Script detection prefers start:dev, then dev, start, serve, watch.",
  },
  {
    title: "3. Launch profiles",
    body: "Save named sets of enabled repos + order (e.g. \"Backend\", \"Everything\") from the Project view, then apply one to launch just that set.",
  },
  {
    title: "4. Execute",
    body: "Starts enabled repositories one at a time, respecting dependency order and the configurable launch delay between each, to avoid resource spikes.",
  },
  {
    title: "5. Monitor & control",
    body: "Track PID, status, and exit code per repository. Start, stop, restart, or open its log stream (with search and filtering) from the Project or Logs view.",
  },
  {
    title: "6. History",
    body: "Every Execute run is recorded — see past launches, their outcomes, and exit codes in the History view.",
  },
];

export function GuideDialog({ onClose }: { onClose: () => void }) {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.4)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
      }}
      onClick={onClose}
    >
      <div
        className="card"
        style={{ width: "100%", maxWidth: 480, maxHeight: "80vh", background: "var(--color-bg)", padding: 24, display: "flex", flexDirection: "column" }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 style={{ fontSize: 18, marginBottom: 6 }}>How to use Multi-Repo Dev Launcher</h2>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 16 }}>
          A quick guide to the core workflow
        </p>

        <div style={{ overflowY: "auto", flex: 1, marginBottom: 16 }}>
          {SECTIONS.map((s) => (
            <div key={s.title} style={{ marginBottom: 16 }}>
              <div style={{ fontWeight: 600, fontSize: 13, marginBottom: 4 }}>{s.title}</div>
              <p className="text-muted" style={{ fontSize: 13, margin: 0, lineHeight: 1.6 }}>{s.body}</p>
            </div>
          ))}
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
