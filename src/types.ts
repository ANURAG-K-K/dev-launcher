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
  removedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectWithRepos {
  project: Project;
  repositories: Repository[];
}
