use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::config::load_config;
use crate::core::embeddings::get_embedder;
use crate::core::index::{build_index, index_status, load_index, index_path};
use crate::core::projects;

#[derive(Parser)]
pub struct IndexArgs {
    #[command(subcommand)]
    pub command: IndexCommands,
}

#[derive(Subcommand)]
pub enum IndexCommands {
    /// Rebuild index for a project (embeds all files)
    Rebuild {
        #[arg(long)]
        project: Option<String>,
        /// Force rebuild even if cache fresh
        #[arg(long)]
        force: bool,
    },
    /// Show index status
    Status {
        #[arg(long)]
        project: Option<String>,
    },
    /// Clear index cache
    Clear {
        #[arg(long)]
        project: Option<String>,
    },
    /// Show index file path
    Path {
        #[arg(long)]
        project: Option<String>,
    },
}

pub async fn handle(args: IndexArgs) -> Result<()> {
    match args.command {
        IndexCommands::Rebuild { project, force } => {
            let proj = projects::resolve_project(project)?;
            if !force {
                if let Some(idx) = load_index(&proj)? {
                    if !crate::core::index::needs_rebuild(&proj, &idx) {
                        println!("Index is fresh ({} files, {}). Use --force to rebuild.", idx.files.len(), idx.embedder_name);
                        println!("  {}", index_path(&proj)?.display());
                        return Ok(());
                    }
                }
            }
            let cfg = load_config().unwrap_or_default();
            let embedder = get_embedder(&cfg);
            println!("Rebuilding index for {} with {}...", proj.folder_path.as_deref().unwrap_or("?"), embedder.name());
            let idx = build_index(&proj, embedder)?;
            println!("{} Indexed {} files → {}", style("✓").green(), idx.files.len(), index_path(&proj)?.display());
        }
        IndexCommands::Status { project } => {
            let proj = projects::resolve_project(project)?;
            let s = index_status(&proj)?;
            println!("{}", s);
            // Show embedder
            let cfg = load_config().unwrap_or_default();
            println!("Embedder config: {} ({})", cfg.embeddings.provider, cfg.embeddings.model);
            // Try health
            if cfg.embeddings.provider == "ollama" {
                let ollama = crate::core::embeddings::OllamaEmbedder::new(cfg.providers.ollama.url.clone(), "nomic-embed-text".into());
                let ok = ollama.health_check_async().await;
                println!("Ollama embeddings health: {}", if ok { "OK" } else { "UNAVAILABLE" });
            }
        }
        IndexCommands::Clear { project } => {
            let proj = projects::resolve_project(project)?;
            let path = index_path(&proj)?;
            if path.exists() {
                std::fs::remove_file(&path)?;
                println!("Cleared {}", path.display());
            } else {
                println!("No index at {}", path.display());
            }
        }
        IndexCommands::Path { project } => {
            let proj = projects::resolve_project(project)?;
            println!("{}", index_path(&proj)?.display());
        }
    }
    Ok(())
}
