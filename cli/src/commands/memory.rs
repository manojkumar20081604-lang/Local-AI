//! `local-ai memory` — three-tier memory (Phase 3.1).
//!
//! ```bash
//! local-ai memory show --scope project --project MyApp
//! local-ai memory set --scope project --project MyApp --content "uses React+Rust"
//! local-ai memory set --scope user --content "prefers TypeScript, short answers"
//! local-ai memory forget --scope project --project MyApp --filter "old stack"
//! ```

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::{memory as core_memory, projects};

#[derive(Parser)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommands,
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// Show stored memory (read-only, plan-mode safe)
    Show {
        /// Tier: project|user|task
        #[arg(long, default_value = "project")]
        scope: String,
        #[arg(long)]
        project: Option<String>,
        /// Task id (required for --scope task)
        #[arg(long)]
        task_id: Option<String>,
    },
    /// Write/append memory (build-gated; secrets are redacted before save)
    Set {
        #[arg(long, default_value = "project")]
        scope: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        task_id: Option<String>,
        /// Full content to store (replaces unless --append)
        #[arg(long)]
        content: String,
        /// Append one line instead of replacing
        #[arg(long)]
        append: bool,
    },
    /// Forget memory (whole tier, or lines matching --filter)
    Forget {
        #[arg(long, default_value = "project")]
        scope: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        task_id: Option<String>,
        /// Only drop lines containing this substring (case-insensitive)
        #[arg(long)]
        filter: Option<String>,
    },
}

fn parse_scope(s: &str) -> Result<core_memory::MemoryScope> {
    s.parse().map_err(|e: String| anyhow::anyhow!(e))
}

pub async fn handle(args: MemoryArgs) -> Result<()> {
    match args.command {
        MemoryCommands::Show { scope, project, task_id } => {
            let sc = parse_scope(&scope)?;
            // `show` is read-only — no build gate.
            let proj = match project {
                Some(p) => Some(projects::resolve_project(Some(p))?),
                None if sc == core_memory::MemoryScope::User => None,
                None => Some(projects::resolve_project(None)?),
            };
            let text = core_memory::load_memory(sc, proj.as_ref(), task_id.as_deref())?;
            if text.trim().is_empty() {
                println!("{}", style(format!("No {} memory stored yet", sc)).dim());
            } else {
                println!("{}", text);
            }
        }
        MemoryCommands::Set { scope, project, task_id, content, append } => {
            crate::core::config::require_build_mode("memory set")?;
            let sc = parse_scope(&scope)?;
            let proj = match project {
                Some(p) => Some(projects::resolve_project(Some(p))?),
                None if sc == core_memory::MemoryScope::User => None,
                None => Some(projects::resolve_project(None)?),
            };
            // Task scope writes require an explicit task id; project writes need a folder.
            if sc == core_memory::MemoryScope::Task && task_id.is_none() {
                anyhow::bail!("--scope task needs --task-id <id>");
            }
            let path = if append {
                core_memory::append_memory(sc, proj.as_ref(), task_id.as_deref(), &content)?
            } else {
                core_memory::save_memory(sc, proj.as_ref(), task_id.as_deref(), &content)?
            };
            println!("{} {} memory → {}", style("✓").green(), sc, path.display());
            if core_memory::redact_secrets(&content) != content {
                eprintln!("{} secrets redacted before save (paths kept, values dropped)", style("→").dim());
            }
        }
        MemoryCommands::Forget { scope, project, task_id, filter } => {
            crate::core::config::require_build_mode("memory forget")?;
            let sc = parse_scope(&scope)?;
            let proj = match project {
                Some(p) => Some(projects::resolve_project(Some(p))?),
                None if sc == core_memory::MemoryScope::User => None,
                None => Some(projects::resolve_project(None)?),
            };
            let removed = core_memory::forget_memory(sc, proj.as_ref(), task_id.as_deref(), filter.as_deref())?;
            if removed {
                println!("{} forgot {} memory{}", style("✓").green(), sc, filter.map(|f| format!(" (filter: {})", f)).unwrap_or_default());
            } else {
                println!("{}", style("Nothing matched — nothing forgotten").dim());
            }
        }
    }
    Ok(())
}
