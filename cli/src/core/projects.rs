use anyhow::{Context, Result};
use chrono::Utc;
use dirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

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

pub fn projects_file() -> Result<PathBuf> {
    // Cross-platform: Linux ~/.local/share/com.localai.app, macOS ~/Library/Application Support/com.localai.app, Windows %APPDATA%
    // Fall back to dirs::data_dir / config_dir
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .context("Could not determine data directory")?;

    let app_dir = if cfg!(target_os = "macos") {
        base.join("com.localai.app")
    } else if cfg!(target_os = "windows") {
        base.join("com.localai.app")
    } else {
        // Linux: match Tauri's app_data_dir which is $XDG_DATA_HOME/com.localai.app or ~/.local/share/com.localai.app
        base.join("com.localai.app")
    };

    fs::create_dir_all(&app_dir).context("Failed to create app data directory")?;
    Ok(app_dir.join("projects.json"))
}

pub fn read_projects() -> Result<Vec<Project>> {
    let path = projects_file()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path).context("Failed to read projects.json")?;
    if contents.trim().is_empty() {
        return Ok(Vec::new());
    }
    let projects: Vec<Project> =
        serde_json::from_str(&contents).context("Failed to parse projects.json")?;
    Ok(projects)
}

pub fn write_projects(projects: &[Project]) -> Result<()> {
    let path = projects_file()?;
    let json = serde_json::to_string_pretty(projects)?;
    fs::write(&path, json)?;
    Ok(())
}

pub fn find_project(identifier: &str) -> Result<Option<Project>> {
    let projects = read_projects()?;
    // identifier can be id, name, or folder_path
    let found = projects.into_iter().find(|p| {
        p.id == identifier || p.name == identifier || p.folder_path.as_deref() == Some(identifier)
    });
    Ok(found)
}

pub fn resolve_project(project_arg: Option<String>) -> Result<Project> {
    if let Some(arg) = project_arg {
        if let Some(p) = find_project(&arg)? {
            return Ok(p);
        }
        // If arg looks like a path, try canonicalize and match
        let cwd = std::env::current_dir()?;
        let candidate = if std::path::Path::new(&arg).is_absolute() {
            PathBuf::from(&arg)
        } else {
            cwd.join(&arg)
        };
        if candidate.exists() {
            let canon = candidate.canonicalize().unwrap_or(candidate);
            let canon_str = canon.to_string_lossy().to_string();
            if let Some(p) = find_project(&canon_str)? {
                return Ok(p);
            }
            // Auto-create ephemeral project for CLI use if not found but folder exists
            return Ok(Project {
                id: Uuid::new_v4().to_string(),
                name: canon.file_name().unwrap_or_default().to_string_lossy().to_string(),
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
                folder_path: Some(canon_str),
                messages: Vec::new(),
            });
        }
        anyhow::bail!("Project not found: {}", arg);
    }

    // No arg: try current directory as project
    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.canonicalize().unwrap_or(cwd).to_string_lossy().to_string();
    if let Some(p) = find_project(&cwd_str)? {
        return Ok(p);
    }
    // Try to find any project matching cwd
    let projects = read_projects()?;
    if let Some(p) = projects.into_iter().find(|p| p.folder_path.as_deref() == Some(&cwd_str)) {
        return Ok(p);
    }
    // Fallback: use cwd as ephemeral project (no persistence until save)
    Ok(Project {
        id: Uuid::new_v4().to_string(),
        name: std::path::Path::new(&cwd_str)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        folder_path: Some(cwd_str),
        messages: Vec::new(),
    })
}

pub fn save_project(project: Project) -> Result<()> {
    let mut projects = read_projects()?;
    if let Some(existing) = projects.iter_mut().find(|p| p.id == project.id) {
        *existing = project;
    } else {
        projects.push(project);
    }
    write_projects(&projects)
}

pub fn delete_project(project_id: &str) -> Result<()> {
    let mut projects = read_projects()?;
    let before = projects.len();
    projects.retain(|p| p.id != project_id && p.name != project_id);
    if projects.len() == before {
        anyhow::bail!("Project not found: {}", project_id);
    }
    write_projects(&projects)
}

pub fn create_project(name: String, folder_path: Option<String>) -> Result<Project> {
    let now = Utc::now().to_rfc3339();
    let project = Project {
        id: Uuid::new_v4().to_string(),
        name: name.clone(),
        created_at: now.clone(),
        updated_at: now,
        folder_path,
        messages: Vec::new(),
    };
    let mut projects = read_projects()?;
    projects.push(project.clone());
    write_projects(&projects)?;
    Ok(project)
}

pub fn update_project_messages(project_id: &str, messages: Vec<ChatMessage>) -> Result<()> {
    let mut projects = read_projects()?;
    let mut found = false;
    for p in &mut projects {
        if p.id == project_id {
            p.messages = messages.clone();
            p.updated_at = Utc::now().to_rfc3339();
            found = true;
            break;
        }
    }
    if !found {
        // Check if project was ephemeral (not yet saved) - save it now
        // Caller should have saved it; we just warn
        anyhow::bail!("Project not found for message save: {}", project_id);
    }
    write_projects(&projects)
}
