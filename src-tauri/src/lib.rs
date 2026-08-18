use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tauri::Manager;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub folder_path: Option<String>,
    pub messages: Vec<ChatMessage>,
}


fn projects_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {e}"))?;

    fs::create_dir_all(&app_data)
        .map_err(|e| format!("Failed to create app data directory: {e}"))?;

    Ok(app_data.join("projects.json"))
}

fn read_projects(app: &tauri::AppHandle) -> Result<Vec<Project>, String> {
    let path = projects_file(app)?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read projects: {e}"))?;

    if contents.trim().is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str(&contents).map_err(|e| format!("Failed to parse projects.json: {e}"))
}

fn write_projects(app: &tauri::AppHandle, projects: &[Project]) -> Result<(), String> {
    let path = projects_file(app)?;

    let json = serde_json::to_string_pretty(projects)
        .map_err(|e| format!("Failed to serialize projects: {e}"))?;

    fs::write(&path, json).map_err(|e| format!("Failed to save projects: {e}"))
}

#[tauri::command]
fn get_projects(app: tauri::AppHandle) -> Result<Vec<Project>, String> {
    read_projects(&app)
}

#[tauri::command]
fn create_project(
    app: tauri::AppHandle,
    id: String,
    name: String,
    created_at: String,
) -> Result<Project, String> {
    let mut projects = read_projects(&app)?;

    let project = Project {
        id,
        name,
        created_at: created_at.clone(),
        updated_at: created_at,
        folder_path: None,
        messages: Vec::new(),
    };

    projects.push(project.clone());

    write_projects(&app, &projects)?;

    Ok(project)
}

#[tauri::command]
fn save_project(app: tauri::AppHandle, project: Project) -> Result<(), String> {
    let mut projects = read_projects(&app)?;

    if let Some(existing) = projects.iter_mut().find(|p| p.id == project.id) {
        *existing = project;
    } else {
        projects.push(project);
    }

    write_projects(&app, &projects)
}

#[tauri::command]
fn delete_project(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let mut projects = read_projects(&app)?;

    projects.retain(|project| project.id != project_id);

    write_projects(&app, &projects)
}

#[tauri::command]
fn get_project(app: tauri::AppHandle, project_id: String) -> Result<Option<Project>, String> {
    let projects = read_projects(&app)?;

    Ok(projects
        .into_iter()
        .find(|project| project.id == project_id))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectFile {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
}

fn scan_project_directory(
    root: &std::path::Path,
    current: &std::path::Path,
    files: &mut Vec<ProjectFile>,
) -> Result<(), String> {
    let entries = fs::read_dir(current)
        .map_err(|e| format!("Failed to read project directory: {e}"))?;

    for entry in entries {
        let entry = entry
            .map_err(|e| format!("Failed to read directory entry: {e}"))?;

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name == "node_modules"
            || name == ".git"
            || name == "target"
            || name == "dist"
            || name == "build"
            || name == ".next"
            || name == ".idea"
            || name == ".vscode"
        {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map_err(|e| format!("Failed to calculate relative path: {e}"))?;

        let relative_string = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");

        if path.is_dir() {
            files.push(ProjectFile {
                name,
                path: relative_string,
                is_directory: true,
            });

            scan_project_directory(root, &path, files)?;
        } else {
            files.push(ProjectFile {
                name,
                path: relative_string,
                is_directory: false,
            });
        }
    }

    Ok(())
}

#[tauri::command]
fn list_project_files(
    project: Project,
) -> Result<Vec<ProjectFile>, String> {
    let folder_path = project
        .folder_path
        .ok_or_else(|| "Project has no folder attached.".to_string())?;

    let root = std::path::PathBuf::from(&folder_path);

    if !root.exists() {
        return Err("Project folder does not exist.".to_string());
    }

    if !root.is_dir() {
        return Err("Project folder is not a directory.".to_string());
    }

    let mut files = Vec::new();

    scan_project_directory(
        &root,
        &root,
        &mut files,
    )?;

    files.sort_by(|a, b| {
        a.path
            .to_lowercase()
            .cmp(&b.path.to_lowercase())
    });

    Ok(files)
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            Ok(())
        })


.invoke_handler(tauri::generate_handler![
    get_projects,
    create_project,
    save_project,
    delete_project,
    get_project,
    list_project_files,
    read_project_files,
    read_project_file,
    write_project_file,
    delete_project_file,
    run_project_command
])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
#[tauri::command]
fn read_project_file(
    project: Project,
    file_path: String,
) -> Result<String, String> {
    let folder_path = project
        .folder_path
        .ok_or_else(|| "Project has no folder attached.".to_string())?;

    let root = std::path::PathBuf::from(&folder_path);

    if !root.exists() {
        return Err("Project folder does not exist.".to_string());
    }

    let file = root.join(&file_path);

    if !file.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }

    if !file.is_file() {
        return Err(format!("Not a file: {}", file_path));
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve project folder: {e}"))?;

    let canonical_file = file
        .canonicalize()
        .map_err(|e| format!("Failed to resolve file: {e}"))?;

    if !canonical_file.starts_with(&canonical_root) {
        return Err("Access outside the project folder is not allowed.".to_string());
    }

    fs::read_to_string(&canonical_file)
        .map_err(|e| format!("Failed to read file: {e}"))
}


#[tauri::command]
fn write_project_file(
    project: Project,
    file_path: String,
    content: String,
) -> Result<String, String> {
    let folder_path = project
        .folder_path
        .ok_or_else(|| "Project has no folder attached.".to_string())?;

    let root = std::path::PathBuf::from(&folder_path);

    if !root.exists() {
        return Err("Project folder does not exist.".to_string());
    }

    if !root.is_dir() {
        return Err("Project folder is not a directory.".to_string());
    }

    // Reject unsafe paths before joining them to the project root.
    let requested_path = std::path::Path::new(&file_path);

    if requested_path.is_absolute() {
        return Err("Absolute file paths are not allowed.".to_string());
    }

    if requested_path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(
            "Parent directory paths are not allowed.".to_string(),
        );
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|e| {
            format!("Failed to resolve project folder: {e}")
        })?;

    let target = root.join(requested_path);

    let canonical_target = if target.exists() {
        // Existing file.
        let canonical_file = target
            .canonicalize()
            .map_err(|e| {
                format!("Failed to resolve file: {e}")
            })?;

        if !canonical_file.starts_with(&canonical_root) {
            return Err(
                "Access outside the project folder is not allowed."
                    .to_string(),
            );
        }

        canonical_file
    } else {
        // New file.
        let target_parent = target
            .parent()
            .ok_or_else(|| "Invalid file path.".to_string())?;

        // Find the nearest existing parent directory.
        let mut existing_parent = target_parent;

        while !existing_parent.exists() {
            existing_parent = existing_parent
                .parent()
                .ok_or_else(|| {
                    "Failed to find a valid parent directory."
                        .to_string()
                })?;
        }

        let canonical_existing_parent = existing_parent
            .canonicalize()
            .map_err(|e| {
                format!("Failed to resolve parent directory: {e}")
            })?;

        if !canonical_existing_parent.starts_with(&canonical_root) {
            return Err(
                "Access outside the project folder is not allowed."
                    .to_string(),
            );
        }
        
        
          
        // Create only the missing parent directories.
        std::fs::create_dir_all(target_parent)
            .map_err(|e| {
                format!("Failed to create parent directories: {e}")
            })?;

        let canonical_parent = target_parent
            .canonicalize()
            .map_err(|e| {
                format!("Failed to resolve new parent directory: {e}")
            })?;

        if !canonical_parent.starts_with(&canonical_root) {
            return Err(
                "Access outside the project folder is not allowed."
                    .to_string(),
            );
        }

        canonical_parent.join(
            target
                .file_name()
                .ok_or_else(|| "Invalid file name.".to_string())?,
        )
    };

    fs::write(&canonical_target, content)
        .map_err(|e| format!("Failed to write file: {e}"))?;

    Ok(format!(
        "Successfully wrote {}",
        file_path
    ))
}



#[tauri::command]
fn delete_project_file(
    project: Project,
    file_path: String,
) -> Result<String, String> {
    let folder_path = project
        .folder_path
        .ok_or_else(|| "Project has no folder attached.".to_string())?;

    let root = std::path::PathBuf::from(&folder_path);

    if !root.exists() {
        return Err("Project folder does not exist.".to_string());
    }

    if !root.is_dir() {
        return Err("Project folder is not a directory.".to_string());
    }

    // Reject unsafe paths before joining them to the project root.
    let requested_path = std::path::Path::new(&file_path);

    if requested_path.is_absolute() {
        return Err("Absolute file paths are not allowed.".to_string());
    }

    if requested_path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(
            "Parent directory paths are not allowed.".to_string(),
        );
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|e| {
            format!("Failed to resolve project folder: {e}")
        })?;

    let target = root.join(requested_path);

    if !target.exists() {
        return Err(format!(
            "File does not exist: {}",
            file_path
        ));
    }

    if !target.is_file() {
        return Err(format!(
            "Not a file: {}",
            file_path
        ));
    }

    let canonical_file = target
        .canonicalize()
        .map_err(|e| {
            format!("Failed to resolve file: {e}")
        })?;

    if !canonical_file.starts_with(&canonical_root) {
        return Err(
            "Access outside the project folder is not allowed."
                .to_string(),
        );
    }

    std::fs::remove_file(&canonical_file)
        .map_err(|e| {
            format!("Failed to delete file: {e}")
        })?;

    if canonical_file.exists() {
        return Err(format!(
            "Deletion verification failed for {}",
            file_path
        ));
    }

    Ok(format!(
        "Successfully deleted {}",
        file_path
    ))
}


#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub exit_code: Option<i32>,
}

#[tauri::command]
fn run_project_command(
    project: Project,
    command: String,
) -> Result<CommandResult, String> {
    let folder_path = project
        .folder_path
        .ok_or_else(|| "Project has no folder attached.".to_string())?;

    let root = std::path::PathBuf::from(&folder_path);

    if !root.exists() {
        return Err("Project folder does not exist.".to_string());
    }

    if !root.is_dir() {
        return Err("Project folder is not a directory.".to_string());
    }

    let output = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(&root)
        .output()
        .map_err(|e| format!("Failed to execute command: {e}"))?;

    Ok(CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        exit_code: output.status.code(),
    })
}





#[tauri::command]
fn read_project_files(
    project: Project,
) -> Result<Vec<(String, String)>, String> {
    let folder_path = project
        .folder_path
        .ok_or_else(|| "Project has no folder attached.".to_string())?;

    let root = std::path::PathBuf::from(&folder_path);

    if !root.exists() {
        return Err("Project folder does not exist.".to_string());
    }

    if !root.is_dir() {
        return Err("Project folder is not a directory.".to_string());
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve project folder: {e}"))?;

    let mut files = Vec::new();

    fn collect_files(
        root: &std::path::Path,
        current: &std::path::Path,
        files: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        let entries = fs::read_dir(current)
            .map_err(|e| format!("Failed to read directory: {e}"))?;

        for entry in entries {
            let entry = entry
                .map_err(|e| format!("Failed to read directory entry: {e}"))?;

            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // Ignore generated/metadata directories.
            if path.is_dir()
                && matches!(
                    name.as_str(),
                    "node_modules"
                        | ".git"
                        | "target"
                        | "dist"
                        | "build"
                        | ".next"
                        | ".idea"
                        | ".vscode"
                        | "__pycache__"
                        | ".venv"
                        | "venv"
                )
            {
                continue;
            }

            if path.is_dir() {
                collect_files(root, &path, files)?;
                continue;
            }

            // Skip obvious binary/generated files.
            if matches!(
                path.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "ico"
                    | "bmp"
                    | "mp3"
                    | "mp4"
                    | "wav"
                    | "avi"
                    | "mov"
                    | "zip"
                    | "7z"
                    | "rar"
                    | "tar"
                    | "gz"
                    | "pdf"
                    | "exe"
                    | "dll"
                    | "so"
                    | "bin"
                    | "class"
                    | "jar"
            ) {
                continue;
            }

            let content = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(_) => continue,
            };

            // Protect against accidentally loading enormous files.
            if content.len() > 1_000_000 {
                continue;
            }

            let relative = path
                .strip_prefix(root)
                .map_err(|e| format!("Failed to calculate relative path: {e}"))?;

            let relative_path = relative
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");

            files.push((relative_path, content));
        }

        Ok(())
    }

    collect_files(
        &canonical_root,
        &canonical_root,
        &mut files,
    )?;

    files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    Ok(files)
}
