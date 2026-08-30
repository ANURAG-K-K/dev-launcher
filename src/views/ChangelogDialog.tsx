import { CHANGELOG } from "@/data/changelog";

export function ChangelogDialog({ version, onClose }: { version: string; onClose: () => void }) {
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
        <h2 style={{ fontSize: 18, marginBottom: 6 }}>What's new - v{version}</h2>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 16 }}>
          Full history, most recent first
        </p>

        <div style={{ overflowY: "auto", flex: 1, marginBottom: 16 }}>
          {CHANGELOG.map((entry) => (
            <div key={entry.version} style={{ marginBottom: 18 }}>
              <div className="sectiontitle" style={{ marginBottom: 6 }}>
                v{entry.version} - {entry.date}
              </div>
              {entry.major.length > 0 && (
                <div style={{ marginBottom: 8 }}>
                  <div className="text-muted" style={{ fontSize: 11, fontWeight: 600, marginBottom: 2 }}>
                    Major
                  </div>
                  <ul style={{ margin: 0, paddingLeft: 18, fontSize: 13, lineHeight: 1.6 }}>
                    {entry.major.map((c, i) => (
                      <li key={i}>{c}</li>
                    ))}
                  </ul>
                </div>
              )}
              {entry.minor.length > 0 && (
                <div>
                  <div className="text-muted" style={{ fontSize: 11, fontWeight: 600, marginBottom: 2 }}>
                    Minor
                  </div>
                  <ul style={{ margin: 0, paddingLeft: 18, fontSize: 13, lineHeight: 1.6 }}>
                    {entry.minor.map((c, i) => (
                      <li key={i}>{c}</li>
                    ))}
                  </ul>
                </div>
              )}
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
