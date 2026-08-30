import { useEffect, useState } from "react";
import { listLaunchHistory } from "@/api";
import type { LaunchRecord, Project } from "@/types";
import { cn } from "@/lib/utils";

/** History view (F13): past "Execute" launch runs for the open project. */
export function History({ project }: { project: Project | null }) {
  const [records, setRecords] = useState<LaunchRecord[]>([]);

  async function refresh(projectId: number) {
    setRecords(await listLaunchHistory(projectId));
  }

  useEffect(() => {
    if (project) refresh(project.id);
    else setRecords([]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.id]);

  if (!project) {
    return (
      <div style={{ padding: "40px 48px" }}>
        <p className="text-muted">Open a project first.</p>
      </div>
    );
  }

  return (
    <div style={{ padding: "40px 48px" }}>
      <div style={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", marginBottom: 16 }}>
        <div>
          <h1 style={{ fontSize: 32, marginBottom: 6 }}>History</h1>
          <p className="text-muted" style={{ margin: 0 }}>Recent Launch Project runs</p>
        </div>
        <button className="btn btn-secondary" onClick={() => refresh(project.id)}>
          Refresh
        </button>
      </div>

      {records.length === 0 ? (
        <p className="text-muted">No launches yet - use Launch Project on the Project screen.</p>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          {records.map((r) => (
            <div key={r.id} className="card" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <span className="mono text-muted" style={{ fontSize: 12.5 }}>
                  {new Date(r.startedAt).toLocaleString()}
                </span>
                {r.profileName ? <span>{r.profileName}</span> : <span className="text-muted">ad-hoc</span>}
                <span
                  className={cn(
                    "tag",
                    r.status === "completed" ? "tag-good" : r.status === "failed" ? "tag-bad" : "tag-neutral",
                  )}
                  style={{ marginLeft: "auto" }}
                >
                  {r.status}
                </span>
              </div>

              {r.items.length > 0 && (
                <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
                  {r.items.map((item, i) => (
                    <span
                      key={i}
                      className={cn(
                        "tag",
                        item.status === "running"
                          ? "tag-good"
                          : item.status === "crashed"
                            ? "tag-bad"
                            : "tag-neutral",
                      )}
                    >
                      {item.repositoryName}
                      {item.pid != null && <span className="mono text-muted"> · pid {item.pid}</span>}
                      {item.exitCode != null && <span className="mono text-muted"> · exit {item.exitCode}</span>}
                    </span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
