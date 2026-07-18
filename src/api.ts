import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { DependencyEdge, ExecuteResult, Profile, Project, ProjectWithRepos, RepoStatus, Repository } from "./types";

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

export const removeRepository = (repositoryId: number) =>
  invoke<Repository[]>("remove_repository", { repositoryId });

export const updateRepositoryConfig = (input: {
  repositoryId: number;
  packageManager: string;
  command: string;
  args: string;
  envFile: string;
}) => invoke<Repository>("update_repository_config", input);

export const executeProject = (projectId: number, launchDelayMs: number) =>
  invoke<ExecuteResult>("execute_project", { projectId, launchDelayMs });

export const stopAll = (projectId: number) =>
  invoke<void>("stop_all", { projectId });

export const listDependencies = (projectId: number) =>
  invoke<DependencyEdge[]>("list_dependencies", { projectId });

export const setRepositoryDependencies = (repositoryId: number, dependsOn: number[]) =>
  invoke<void>("set_repository_dependencies", { repositoryId, dependsOn });

export const listProfiles = (projectId: number) =>
  invoke<Profile[]>("list_profiles", { projectId });

export const createProfile = (projectId: number, name: string, launchDelayMs: number, repositoryIds: number[]) =>
  invoke<Profile>("create_profile", { projectId, name, launchDelayMs, repositoryIds });

export const updateProfile = (profileId: number, name: string, launchDelayMs: number, repositoryIds: number[]) =>
  invoke<Profile>("update_profile", { profileId, name, launchDelayMs, repositoryIds });

export const deleteProfile = (profileId: number) =>
  invoke<void>("delete_profile", { profileId });

export const applyProfile = (profileId: number) =>
  invoke<Repository[]>("apply_profile", { profileId });
