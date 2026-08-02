import { useEffect, useRef, useState, type CSSProperties } from "react";
import { gitListBranches } from "@/api";
import type { BranchEntry, Repository, RepoGitStatus } from "@/types";

const INPUT_STYLE: CSSProperties = {
  width: "100%",
  padding: "8px 10px",
  border: "1px solid var(--color-divider)",
  background: "var(--color-bg)",
  borderRadius: "var(--radius-md)",
};

/** "HEAD" is what git reports for `--abbrev-ref HEAD` in detached-HEAD state. */
function displayBranch(branch: string): string {
  return branch === "HEAD" ? "Detached HEAD" : branch;
}

/**
 * Modal to switch a repository's branch (spec 2026-08-02). Branch list is fetched fresh on
 * every open/refresh — never cached, unlike the git-status cache. The actual switch call is
 * owned by the parent (`onSwitch`) so it can route through the existing per-repo `runAction`
 * busy-state gate shared with Fetch/Pull/Start/Stop.
 */
export function BranchSwitchDialog({
  repo,
  currentStatus,
  onClose,
  onSwitch,
  switching,
}: {
  repo: Repository;
  currentStatus: RepoGitStatus;
  onClose: () => void;
  onSwitch: (branch: string, stash: boolean, stashMessage: string, stashUntracked: boolean) => Promise<void>;
  switching: boolean;
}) {
  const [branches, setBranches] = useState<BranchEntry[] | null>(null);
  const [loadError, setLoadError] = useState("");
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [stashMessage, setStashMessage] = useState("");
  const [stashUntracked, setStashUntracked] = useState(false);
  const [switchError, setSwitchError] = useState("");
  const cancelled = useRef(false);

  useEffect(() => {
    cancelled.current = false;
    return () => {
      cancelled.current = true;
    };
  }, []);

  async function load() {
    setLoadError("");
    try {
      const result = await gitListBranches(repo.id);
      if (cancelled.current) return;
      setBranches(result);
      // Preserve selection if it still exists in the refreshed list; otherwise clear it.
      setSelected((prev) => (prev && result.some((b) => b.name === prev) ? prev : null));
    } catch (err) {
      if (cancelled.current) return;
      setLoadError(String(err));
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo.id]);

  const filtered = (branches ?? []).filter((b) =>
    b.name.toLowerCase().includes(filter.toLowerCase())
  );

  async function handleSwitch() {
    if (!selected) return;
    setSwitchError("");
    try {
      await onSwitch(selected, currentStatus.dirty, stashMessage, stashUntracked);
      if (!cancelled.current) onClose();
    } catch (err) {
      if (!cancelled.current) setSwitchError(String(err));
    }
  }

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
      onClick={() => {
        if (!switching) onClose();
      }}
    >
      <div
        className="card"
        style={{ width: "100%", maxWidth: 420, background: "var(--color-bg)", padding: 24 }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 style={{ fontSize: 18, marginBottom: 6 }}>Switch branch — {repo.name}</h2>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 16 }}>
          Current: {displayBranch(currentStatus.branch)}
        </p>

        <div style={{ display: "flex", gap: 8, marginBottom: 10 }}>
          <input
            style={{ ...INPUT_STYLE, flex: 1 }}
            placeholder="Filter branches…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <button type="button" className="btn btn-secondary" onClick={() => void load()}>
            Refresh
          </button>
        </div>

        {loadError && (
          <div style={{ color: "var(--color-status-bad-fg)", fontSize: 12, marginBottom: 12 }}>
            {loadError}
          </div>
        )}

        {branches === null && !loadError ? (
          <div className="text-muted" style={{ fontSize: 12, marginBottom: 16 }}>Loading branches…</div>
        ) : branches !== null && branches.length <= 1 ? (
          <div className="text-muted" style={{ fontSize: 12, marginBottom: 16 }}>
            No other branches to switch to.
          </div>
        ) : filtered.length === 0 ? (
          <div className="text-muted" style={{ fontSize: 12, marginBottom: 16 }}>
            No branches match this filter.
          </div>
        ) : (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 2,
              maxHeight: 220,
              overflowY: "auto",
              marginBottom: 16,
            }}
          >
            {filtered.map((b) => (
              <label
                key={b.name}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  padding: "6px 8px",
                  borderRadius: "var(--radius-sm)",
                  background: selected === b.name ? "var(--color-accent-bg, rgba(0,0,0,0.05))" : undefined,
                  opacity: b.isCurrent ? 0.5 : 1,
                  cursor: b.isCurrent ? "default" : "pointer",
                }}
              >
                <input
                  type="radio"
                  name="branch"
                  checked={selected === b.name}
                  disabled={b.isCurrent}
                  onChange={() => setSelected(b.name)}
                />
                <span className="mono">{displayBranch(b.name)}</span>
                {b.isCurrent && <span className="text-muted" style={{ fontSize: 11 }}>(current)</span>}
              </label>
            ))}
          </div>
        )}

        {currentStatus.dirty && (
          <div style={{ marginBottom: 16, padding: 10, border: "1px solid var(--color-divider)", borderRadius: "var(--radius-md)" }}>
            <div className="sectiontitle" style={{ marginBottom: 8 }}>Uncommitted changes will be stashed</div>
            <input
              style={{ ...INPUT_STYLE, marginBottom: 8 }}
              placeholder="Stash message (optional)"
              value={stashMessage}
              onChange={(e) => setStashMessage(e.target.value)}
            />
            <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
              <input
                type="checkbox"
                checked={stashUntracked}
                onChange={(e) => setStashUntracked(e.target.checked)}
              />
              Include untracked files
            </label>
          </div>
        )}

        {switchError && (
          <div style={{ color: "var(--color-status-bad-fg)", fontSize: 12, marginBottom: 12 }}>
            {switchError}
            {switchError.toLowerCase().includes("uncommitted changes") &&
              " Close and reopen this dialog to see stash options."}
          </div>
        )}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 8 }}>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={switching}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void handleSwitch()}
            disabled={!selected || switching}
          >
            {switching ? "Switching…" : "Switch"}
          </button>
        </div>
      </div>
    </div>
  );
}
