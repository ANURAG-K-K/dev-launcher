import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { DependencyEdge, ExecuteResult, LaunchRecord, Profile, Project, ProjectWithRepos, RepoGitStatus, RepoStatus, Repository, Settings } from "./types";

/** Native folder picker; returns the chosen absolute path, or null if cancelled. */
export async function pickDirectory(): Promise<string | null> {
  const res = await openDialog({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}

export const openProject = (rootPath: string) =>
  invoke<ProjectWithRepos>("open_project", { rootPath });

export const scanRepositories = (projectId: number) =>
  invoke<ProjectWithRepos>("scan_repositories", { projectId });

export const gitStatusProject = (projectId: number) =>
  invoke<RepoGitStatus[]>("git_status_project", { projectId });

export const gitFetch = (repositoryId: number) =>
  invoke<string>("git_fetch", { repositoryId });

export const gitPull = (repositoryId: number) =>
  invoke<string>("git_pull", { repositoryId });

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

export const setRepositoryFavorite = (repositoryId: number, favorite: boolean) =>
  invoke<Repository>("set_repository_favorite", { repositoryId, favorite });

export const setRepositoryVisibleConsole = (repositoryId: number, visibleConsole: boolean) =>
  invoke<Repository>("set_repository_visible_console", { repositoryId, visibleConsole });

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

export const openRepoFolder = (repositoryId: number) =>
  invoke<void>("open_repo_folder", { repositoryId });

export const openRepoTerminal = (repositoryId: number) =>
  invoke<void>("open_repo_terminal", { repositoryId });

export const listLaunchHistory = (projectId: number) =>
  invoke<LaunchRecord[]>("list_launch_history", { projectId });

export const getSettings = () => invoke<Settings>("get_settings");

export const updateSettings = (settings: Settings) =>
  invoke<Settings>("update_settings", { settings });
