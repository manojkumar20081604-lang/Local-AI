use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::{fs as core_fs, projects};

#[derive(Parser)]
pub struct FilesArgs {
    #[command(subcommand)]
    pub command: FilesCommands,
}

#[derive(Subcommand)]
pub enum FilesCommands {
    /// List project files (recursive)
    List {
        #[arg(long)]
        project: Option<String>,
        /// Show as JSON
        #[arg(long)]
        json: bool,
    },
    /// Read a file
    Read {
        path: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Write/create a file
    Write {
        path: String,
        #[arg(long)]
        project: Option<String>,
        /// Content string (if not provided, reads from stdin or --file)
        #[arg(long)]
        content: Option<String>,
        /// Read content from file
        #[arg(long)]
        file: Option<String>,
    },
    /// Delete a file
    Delete {
        path: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        force: bool,
    },
}

pub async fn handle(args: FilesArgs) -> Result<()> {
    match args.command {
        FilesCommands::List { project, json } => {
            let proj = projects::resolve_project(project)?;
            let files = core_fs::list_project_files(&proj)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&files)?);
            } else {
                for f in &files {
                    let icon = if f.is_directory { "📁" } else { "📄" };
                    println!("{} {}", icon, f.path);
                }
                println!("\n{} items (project: {})", files.len(), proj.folder_path.unwrap_or_default());
            }
        }
        FilesCommands::Read { path, project } => {
            let proj = projects::resolve_project(project)?;
            let content = core_fs::read_project_file(&proj, &path)?;
            println!("{}", content);
        }
        FilesCommands::Write { path, project, content, file } => {
            crate::core::config::require_build_mode("files write")?;
            let proj = projects::resolve_project(project)?;
            let data = if let Some(c) = content {
                c
            } else if let Some(f) = file {
                std::fs::read_to_string(&f)?
            } else {
                // stdin
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            };
            let msg = core_fs::write_project_file(&proj, &path, &data)?;
            println!("{} {}", style("✓").green(), msg);
        }
        FilesCommands::Delete { path, project, force } => {
            crate::core::config::require_build_mode("files delete")?;
            let proj = projects::resolve_project(project)?;
            if !force {
                println!("Delete {} ?", style(&path).red());
                if !dialoguer::Confirm::new().with_prompt("Confirm?").default(false).interact()? {
                    println!("Aborted");
                    return Ok(());
                }
            }
            let msg = core_fs::delete_project_file(&proj, &path)?;
            println!("{} {}", style("✓").green(), msg);
        }
    }
    Ok(())
}
