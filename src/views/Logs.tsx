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
  const [search, setSearch] = useState("");
  const [streamFilter, setStreamFilter] = useState<"all" | "stdout" | "stderr">("all");
  const [autoScroll, setAutoScroll] = useState(true);
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
  const query = search.trim().toLowerCase();
  const filteredLines = selectedLines.filter(
    (l) => (streamFilter === "all" || l.stream === streamFilter) && (query === "" || l.line.toLowerCase().includes(query)),
  );

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [filteredLines.length, autoScroll]);

  function clearSelectedRepo() {
    if (selectedRepoId == null) return;
    setLinesByRepo((prev) => ({ ...prev, [selectedRepoId]: [] }));
  }

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
            style={{ background: "var(--color-surface)", color: "var(--color-text)" }}
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

      {selectedRepoId != null && (
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 10, flexWrap: "wrap" }}>
          <input
            type="text"
            className="mono"
            placeholder="Filter logs…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            style={{
              fontSize: 12.5,
              padding: "6px 10px",
              minWidth: 220,
              background: "var(--color-terminal-bg)",
              color: "var(--color-terminal-fg)",
              border: "1px solid var(--color-terminal-border)",
              borderRadius: 6,
            }}
          />
          <select
            className="btn btn-secondary"
            value={streamFilter}
            onChange={(e) => setStreamFilter(e.target.value as "all" | "stdout" | "stderr")}
            style={{ background: "var(--color-surface)", color: "var(--color-text)" }}
          >
            <option value="all">All streams</option>
            <option value="stdout">stdout</option>
            <option value="stderr">stderr</option>
          </select>
          <label className="text-muted" style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 13 }}>
            <input type="checkbox" checked={autoScroll} onChange={(e) => setAutoScroll(e.target.checked)} />
            Auto-scroll
          </label>
          <button type="button" className="btn btn-secondary" onClick={clearSelectedRepo}>
            Clear
          </button>
          <span className="text-muted" style={{ fontSize: 13, marginLeft: "auto" }}>
            {selectedLines.length} lines · {filteredLines.length} shown
          </span>
        </div>
      )}

      <div
        ref={scrollRef}
        style={{
          flex: 1,
          background: "var(--color-terminal-bg)",
          color: "var(--color-terminal-fg)",
          padding: "18px 20px",
          overflow: "auto",
        }}
      >
        {selectedRepoId == null ? (
          <p className="mono" style={{ fontSize: 12.5, opacity: 0.5, margin: 0 }}>
            Per-repository live log streaming will live here (F11). Scaffold placeholder.
          </p>
        ) : filteredLines.length === 0 ? (
          <p className="mono" style={{ fontSize: 12.5, opacity: 0.5, margin: 0 }}>
            {selectedLines.length === 0 ? "No output yet." : "No lines match the current filter."}
          </p>
        ) : (
          filteredLines.map((l, i) => (
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
