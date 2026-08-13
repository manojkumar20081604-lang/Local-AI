import { invoke } from "@tauri-apps/api/core";

export interface StoredChatMessage {
  role: "user" | "assistant";
  content: string;
}

export interface StoredProject {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  folder_path: string | null;
  messages: StoredChatMessage[];
}

export interface ProjectFile {
  name: string;
  path: string;
  is_directory: boolean;
}

export async function getProjects(): Promise<StoredProject[]> {
  return invoke<StoredProject[]>("get_projects");
}

export async function getProject(
  projectId: string,
): Promise<StoredProject | null> {
  return invoke<StoredProject | null>("get_project", {
    projectId,
  });
}

export async function createProject(
  id: string,
  name: string,
  createdAt: string,
): Promise<StoredProject> {
  return invoke<StoredProject>("create_project", {
    id,
    name,
    createdAt,
  });
}

export async function saveProject(
  project: StoredProject,
): Promise<void> {
  await invoke("save_project", {
    project,
  });
}

export async function deleteProject(
  projectId: string,
): Promise<void> {
  await invoke("delete_project", {
    projectId,
  });
}

export async function listProjectFiles(
  project: StoredProject,
): Promise<ProjectFile[]> {
  return invoke<ProjectFile[]>("list_project_files", {
    project,
  });
}

export async function readProjectFiles(
  project: StoredProject,
): Promise<Array<[string, string]>> {
  return invoke<Array<[string, string]>>("read_project_files", {
    project,
  });
}


export async function readProjectFile(
  project: StoredProject,
  filePath: string,
): Promise<string> {
  return invoke<string>("read_project_file", {
    project,
    filePath,
  });
}

export async function deleteProjectFile(
  project: StoredProject,
  filePath: string,
): Promise<string> {
  return invoke<string>("delete_project_file", {
    project,
    filePath,
  });
}


export interface CommandResult {
  stdout: string;
  stderr: string;
  success: boolean;
  exit_code: number | null;
}

export async function runProjectCommand(
  project: StoredProject,
  command: string,
): Promise<CommandResult> {
  return invoke<CommandResult>("run_project_command", {
    project,
    command,
  });
}


export async function writeProjectFile(
  project: StoredProject,
  filePath: string,
  content: string,
): Promise<string> {
  return invoke<string>("write_project_file", {
    project,
    filePath,
    content,
  });
}
