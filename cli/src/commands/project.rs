use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::projects;

#[derive(Parser)]
pub struct ProjectArgs {
    #[command(subcommand)]
    pub command: ProjectCommands,
}

#[derive(Subcommand)]
pub enum ProjectCommands {
    /// List all projects
    List,
    /// Show project details
    Show { id: String },
    /// Create a new project (attach a folder)
    Create {
        /// Project name
        name: String,
        /// Folder path to attach (default: current dir)
        #[arg(long)]
        path: Option<String>,
    },
    /// Attach current or specified folder as project
    Attach {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Delete a project (does NOT delete folder)
    Delete { id: String, #[arg(long)] force: bool },
    /// Show where projects.json is stored
    Info,
}

pub async fn handle(args: ProjectArgs) -> Result<()> {
    match args.command {
        ProjectCommands::List => {
            let projects = projects::read_projects()?;
            if projects.is_empty() {
                println!("No projects. Use `local-ai project attach .` or `local-ai project create MyProj --path ./folder`");
                return Ok(());
            }
            println!("{:<36} {:<20} {:<40} {}", "ID", "NAME", "FOLDER", "UPDATED");
            for p in projects {
                println!(
                    "{:<36} {:<20} {:<40} {}",
                    p.id,
                    style(&p.name).cyan(),
                    p.folder_path.unwrap_or_else(|| "-".to_string()),
                    p.updated_at
                );
            }
        }
        ProjectCommands::Show { id } => {
            let p = projects::find_project(&id)?.ok_or_else(|| anyhow::anyhow!("Project not found: {}", id))?;
            println!("{}", serde_json::to_string_pretty(&p)?);
        }
        ProjectCommands::Create { name, path } => {
            let folder = if let Some(p) = path {
                let pb = std::path::PathBuf::from(&p);
                let canon = pb.canonicalize().unwrap_or(pb);
                Some(canon.to_string_lossy().to_string())
            } else {
                None
            };
            let proj = projects::create_project(name, folder)?;
            println!("Created project {} ({})", style(&proj.name).green(), proj.id);
            if let Some(fp) = &proj.folder_path {
                println!("  folder: {}", fp);
            }
        }
        ProjectCommands::Attach { path, name } => {
            let pb = std::path::PathBuf::from(&path);
            let canon = pb.canonicalize().map_err(|_| anyhow::anyhow!("Folder does not exist: {}", path))?;
            let canon_str = canon.to_string_lossy().to_string();
            let proj_name = name.unwrap_or_else(|| canon.file_name().unwrap_or_default().to_string_lossy().to_string());
            // check if already exists
            if let Some(existing) = projects::find_project(&canon_str)? {
                println!("Already attached as {} ({})", existing.name, existing.id);
                return Ok(());
            }
            let proj = projects::create_project(proj_name, Some(canon_str))?;
            println!("Attached {} ({})", style(&proj.name).green(), proj.id);
            println!("  folder: {}", proj.folder_path.unwrap());
        }
        ProjectCommands::Delete { id, force } => {
            let proj = projects::find_project(&id)?.ok_or_else(|| anyhow::anyhow!("Project not found: {}", id))?;
            if !force {
                println!("Delete project \"{}\" ({})? This will NOT delete the folder.", proj.name, proj.id);
                // Handle non-tty gracefully
                let confirm = dialoguer::Confirm::new().with_prompt("Confirm delete?").default(false).interact_opt()?;
                if confirm != Some(true) {
                    println!("Aborted (use --force to skip prompt)");
                    return Ok(());
                }
            }
            projects::delete_project(&proj.id)?;
            println!("Deleted {}", proj.id);
        }
        ProjectCommands::Info => {
            let path = projects::projects_file()?;
            println!("projects.json: {}", path.display());
            let projects = projects::read_projects()?;
            println!("projects: {}", projects.len());
        }
    }
    Ok(())
}
