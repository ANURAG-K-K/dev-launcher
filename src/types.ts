/** Mirrors the Rust `persistence::Project` (serde camelCase). */
export interface Project {
  id: number;
  name: string;
  rootPath: string;
  lastOpenedAt: string;
  lastProfileId: number | null;
  createdAt: string;
  updatedAt: string;
}

/** Mirrors the Rust `persistence::Repository` (serde camelCase). */
export interface Repository {
  id: number;
  projectId: number;
  name: string;
  path: string;
  packageManager: string;
  detectedScript: string | null;
  command: string | null;
  args: string | null;
  envFile: string | null;
  enabled: number; // 0 | 1
  favorite: number; // 0 | 1
  visibleConsole: number; // 0 | 1
  removedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectWithRepos {
  project: Project;
  repositories: Repository[];
}

/** Payload of the `repo_status_changed` event; also the return type of start/restart_repo. */
export interface RepoStatus {
  repositoryId: number;
  status: "starting" | "running" | "stopped" | "crashed";
  pid: number | null;
  exitCode: number | null;
  restartCount: number;
}

/** Payload of the `repo_log` event. */
export interface RepoLogLine {
  repositoryId: number;
  stream: "stdout" | "stderr";
  line: string;
  timestamp: string;
}

/** A dependency edge: repositoryId depends on dependsOnRepositoryId. */
export interface DependencyEdge {
  repositoryId: number;
  dependsOnRepositoryId: number;
}

/** Per-repository git status; absent from `gitStatusProject`'s result means "not a git repo". */
export interface RepoGitStatus {
  repositoryId: number;
  branch: string;
  dirty: boolean;
  ahead: number;
  behind: number;
}

/** A branch available to switch to; `isCurrent` is true for at most one entry. */
export interface BranchEntry {
  name: string;
  isCurrent: boolean;
}

export interface ExecuteResult {
  started: number[];
  skipped: number[];
}

/** A saved selection of repositories + a launch delay ("Launch Profile"). */
export interface Profile {
  id: number;
  name: string;
  launchDelayMs: number;
  repositoryIds: number[];
}

/** An entry in a launch record's `items` list (one per repository). */
export interface LaunchItemRecord {
  repositoryName: string;
  status: string;
  pid: number | null;
  exitCode: number | null;
  stoppedAt: string | null;
}

/** A recorded "Execute" run. */
export interface LaunchRecord {
  id: number;
  profileName: string | null;
  startedAt: string; // ISO-8601
  finishedAt: string | null;
  status: string; // "running" | "completed" | "failed"
  items: LaunchItemRecord[];
}

/** Mirrors the Rust `Settings` (serde camelCase). */
export interface Settings {
  theme: "light" | "dark" | "system";
  launchDelayMs: number;
  autoDetect: boolean;
  restoreLastProject: boolean;
  restoreLastSelection: boolean;
  autoRestart: boolean;
  logRetention: number;
  notificationsEnabled: boolean;
  terminalShell: "cmd" | "powershell";
  visibleActions: string[];
}
