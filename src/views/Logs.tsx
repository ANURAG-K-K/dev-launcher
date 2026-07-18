import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { RepoLogLine, Repository } from "@/types";

const MAX_LINES_PER_REPO = 1000;

/** Logs view (F11): per-repo live log viewer subscribed to repo_log events. */
export function Logs({
  repositories,
  selectedRepoId,
  onSelectRepo,
}: {
  repositories: Repository[];
  selectedRepoId: number | null;
  onSelectRepo: (repositoryId: number) => void;
}) {
  const [linesByRepo, setLinesByRepo] = useState<Record<number, RepoLogLine[]>>({});
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<RepoLogLine>("repo_log", (event) => {
      const line = event.payload;
      setLinesByRepo((prev) => {
        const existing = prev[line.repositoryId] ?? [];
        const next = [...existing, line];
        if (next.length > MAX_LINES_PER_REPO) {
          next.splice(0, next.length - MAX_LINES_PER_REPO);
        }
        return { ...prev, [line.repositoryId]: next };
      });
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  const selectedLines = selectedRepoId != null ? linesByRepo[selectedRepoId] ?? [] : [];

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [selectedLines.length]);

  return (
    <div style={{ padding: "40px 48px", display: "flex", flexDirection: "column", height: "100vh", boxSizing: "border-box" }}>
      <h1 style={{ fontSize: 32, marginBottom: 6 }}>Logs</h1>
      <p className="text-muted" style={{ margin: "0 0 16px" }}>Live process output, one stream per repository</p>

      {repositories.length > 0 && (
        <div style={{ marginBottom: 16 }}>
          <select
            className="btn btn-secondary"
            value={selectedRepoId ?? ""}
            onChange={(e) => onSelectRepo(Number(e.target.value))}
          >
            <option value="" disabled>
              Select a repository…
            </option>
            {repositories.map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
        </div>
      )}

      <div
        ref={scrollRef}
        style={{
          flex: 1,
          background: "var(--color-neutral-900)",
          color: "var(--color-neutral-100)",
          padding: "18px 20px",
          overflow: "auto",
        }}
      >
        {selectedRepoId == null ? (
          <p className="mono" style={{ fontSize: 12.5, opacity: 0.5, margin: 0 }}>
            Per-repository live log streaming will live here (F11). Scaffold placeholder.
          </p>
        ) : selectedLines.length === 0 ? (
          <p className="mono" style={{ fontSize: 12.5, opacity: 0.5, margin: 0 }}>
            No output yet.
          </p>
        ) : (
          selectedLines.map((l, i) => (
            <div
              key={i}
              className="mono"
              style={{
                fontSize: 12.5,
                whiteSpace: "pre-wrap",
                color: l.stream === "stderr" ? "var(--color-status-bad-fg)" : undefined,
              }}
            >
              {l.line}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
