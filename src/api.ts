import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { Project, ProjectWithRepos, RepoStatus, Repository } from "./types";

/** Native folder picker; returns the chosen absolute path, or null if cancelled. */
export async function pickDirectory(): Promise<string | null> {
  const res = await openDialog({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}

export const openProject = (rootPath: string) =>
  invoke<ProjectWithRepos>("open_project", { rootPath });

export const scanRepositories = (projectId: number) =>
  invoke<ProjectWithRepos>("scan_repositories", { projectId });

export const listRecentProjects = () =>
  invoke<Project[]>("list_recent_projects");

export const startRepo = (repositoryId: number) =>
  invoke<RepoStatus>("start_repo", { repositoryId });

export const stopRepo = (repositoryId: number) =>
  invoke<void>("stop_repo", { repositoryId });

export const restartRepo = (repositoryId: number) =>
  invoke<RepoStatus>("restart_repo", { repositoryId });

export const setRepositoryEnabled = (repositoryId: number, enabled: boolean) =>
  invoke<Repository>("set_repository_enabled", { repositoryId, enabled });
