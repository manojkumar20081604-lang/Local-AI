use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use super::projects::Project;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectFile {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
}

fn ignored_dir(name: &str) -> bool {
    matches!(
        name,
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
            | ".cache"
            | "coverage"
            | ".fastembed_cache"
            | ".huggingface"
    ) || name.starts_with(".fastembed")
        || name.starts_with("models--")
}

fn scan_dir(root: &Path, current: &Path, files: &mut Vec<ProjectFile>) -> Result<()> {
    let entries = fs::read_dir(current).with_context(|| format!("Failed to read {}", current.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if ignored_dir(&name) {
            continue;
        }
        let relative = path.strip_prefix(root).context("Failed to calc relative path")?;
        let rel_str = relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        if path.is_dir() {
            files.push(ProjectFile {
                name: name.clone(),
                path: rel_str.clone(),
                is_directory: true,
            });
            scan_dir(root, &path, files)?;
        } else {
            files.push(ProjectFile {
                name,
                path: rel_str,
                is_directory: false,
            });
        }
    }
    Ok(())
}

pub fn list_project_files(project: &Project) -> Result<Vec<ProjectFile>> {
    let folder = project
        .folder_path
        .as_ref()
        .context("Project has no folder attached")?;
    let root = PathBuf::from(folder);
    if !root.exists() {
        anyhow::bail!("Project folder does not exist: {}", folder);
    }
    if !root.is_dir() {
        anyhow::bail!("Project folder is not a directory: {}", folder);
    }
    let mut files = Vec::new();
    scan_dir(&root, &root, &mut files)?;
    files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    Ok(files)
}

fn resolve_safe_path(project: &Project, file_path: &str) -> Result<PathBuf> {
    let folder = project
        .folder_path
        .as_ref()
        .context("Project has no folder attached")?;
    let root = PathBuf::from(folder);
    let requested = Path::new(file_path);
    if requested.is_absolute() {
        anyhow::bail!("Absolute paths not allowed: {}", file_path);
    }
    if requested.components().any(|c| c == Component::ParentDir) {
        anyhow::bail!("Parent directory '..' not allowed: {}", file_path);
    }
    Ok(root.join(requested))
}

fn ensure_inside_root(root: &Path, target: &Path) -> Result<PathBuf> {
    let canonical_root = root.canonicalize().context("Failed to resolve project folder")?;
    // For existing file, canonicalize directly; for new file, canonicalize parent
    let canonical_target = if target.exists() {
        target.canonicalize().context("Failed to resolve target")?
    } else {
        let parent = target.parent().context("Invalid file path")?;
        // Find nearest existing parent
        let mut existing = parent;
        while !existing.exists() {
            existing = existing.parent().context("No valid parent")?;
        }
        let canon_parent = existing.canonicalize().context("Failed to resolve parent")?;
        if !canon_parent.starts_with(&canonical_root) {
            anyhow::bail!("Access outside project folder not allowed");
        }
        // Create missing dirs
        fs::create_dir_all(parent).context("Failed to create parent dirs")?;
        let canon_parent2 = parent.canonicalize().context("Failed to resolve new parent")?;
        if !canon_parent2.starts_with(&canonical_root) {
            anyhow::bail!("Access outside project folder not allowed");
        }
        canon_parent2.join(target.file_name().context("Invalid file name")?)
    };
    // For existing case, also check prefix
    if target.exists() && !canonical_target.starts_with(&canonical_root) {
        anyhow::bail!("Access outside project folder not allowed");
    }
    Ok(canonical_target)
}

pub fn read_project_file(project: &Project, file_path: &str) -> Result<String> {
    let root = PathBuf::from(project.folder_path.as_ref().context("No folder")?);
    let target = resolve_safe_path(project, file_path)?;
    if !target.exists() {
        anyhow::bail!("File does not exist: {}", file_path);
    }
    if !target.is_file() {
        anyhow::bail!("Not a file: {}", file_path);
    }
    let canonical = ensure_inside_root(&root, &target)?;
    fs::read_to_string(&canonical).with_context(|| format!("Failed to read {}", file_path))
}

pub fn write_project_file(project: &Project, file_path: &str, content: &str) -> Result<String> {
    let root = PathBuf::from(project.folder_path.as_ref().context("No folder")?);
    let target = resolve_safe_path(project, file_path)?;
    let canonical = ensure_inside_root(&root, &target)?;
    fs::write(&canonical, content).with_context(|| format!("Failed to write {}", file_path))?;
    Ok(format!("Wrote {}", file_path))
}

pub fn delete_project_file(project: &Project, file_path: &str) -> Result<String> {
    let root = PathBuf::from(project.folder_path.as_ref().context("No folder")?);
    let target = resolve_safe_path(project, file_path)?;
    if !target.exists() {
        anyhow::bail!("File does not exist: {}", file_path);
    }
    if !target.is_file() {
        anyhow::bail!("Not a file: {}", file_path);
    }
    let canonical = ensure_inside_root(&root, &target)?;
    fs::remove_file(&canonical).with_context(|| format!("Failed to delete {}", file_path))?;
    if canonical.exists() {
        anyhow::bail!("Deletion verification failed: {}", file_path);
    }
    Ok(format!("Deleted {}", file_path))
}

#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub exit_code: Option<i32>,
}

pub fn run_project_command(project: &Project, command: &str) -> Result<CommandResult> {
    let folder = project.folder_path.as_ref().context("No folder")?;
    let root = PathBuf::from(folder);
    if !root.exists() {
        anyhow::bail!("Project folder does not exist");
    }
    // Cross-platform shell: use sh on unix, cmd on windows
    let output = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", command])
            .current_dir(&root)
            .output()
            .context("Failed to execute command")?
    } else {
        Command::new("sh")
            .args(["-c", command])
            .current_dir(&root)
            .output()
            .context("Failed to execute command")?
    };
    Ok(CommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        exit_code: output.status.code(),
    })
}

pub fn collect_files_for_finetune(project: &Project) -> Result<Vec<(String, String)>> {
    let folder = project.folder_path.as_ref().context("No folder")?;
    let root = PathBuf::from(folder).canonicalize()?;
    let mut files = Vec::new();
    collect_recursive(&root, &root, &mut files)?;
    files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    Ok(files)
}

fn collect_recursive(root: &Path, current: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
    let entries = fs::read_dir(current)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() && ignored_dir(&name) {
            continue;
        }
        if path.is_dir() {
            collect_recursive(root, &path, out)?;
            continue;
        }
        // Skip binaries
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" | "mp3" | "mp4" | "wav" | "avi" | "mov" | "zip" | "7z" | "rar" | "tar" | "gz" | "pdf" | "exe" | "dll" | "so" | "bin" | "class" | "jar" | "o" | "a"
        ) {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if content.len() > 1_000_000 {
            continue;
        }
        let rel = path.strip_prefix(root)?.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        out.push((rel, content));
    }
    Ok(())
}
