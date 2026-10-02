//! `local-ai dataset collect` — approved interactions → QLoRA JSONL (Phase 5.2).
//!
//! ```bash
//! local-ai dataset collect --project MyApp --from missions --only-approved --out ./dataset.jsonl
//! local-ai dataset collect --project MyApp --from all --format alpaca --out ./dataset.jsonl
//! ```
//!
//! Output reuses the `finetune prepare` ShareGPT format, so `finetune train`
//! consumes it unchanged. Finetuning never bypasses grounding: collected
//! samples are verifier-filtered with `--only-approved`, and the verifier
//! still runs post-deploy on every model output.

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

use crate::core::{dataset as core_dataset, fs as core_fs, missions as core_missions, projects, verifier};

#[derive(Parser)]
pub struct DatasetArgs {
    #[command(subcommand)]
    pub command: DatasetCommands,
}

#[derive(Subcommand)]
pub enum DatasetCommands {
    /// Collect approved interactions into a training JSONL file
    Collect {
        /// Project id/name/path (default: current dir)
        #[arg(long)]
        project: Option<String>,
        /// Sources: chat|missions|all (default: all)
        #[arg(long, default_value = "all")]
        from: String,
        /// Keep only verifier-clean samples
        #[arg(long)]
        only_approved: bool,
        /// Output JSONL (appends when the file exists)
        #[arg(long, default_value = "./finetune-dataset.jsonl")]
        out: PathBuf,
        /// Format: sharegpt|alpaca (default: sharegpt)
        #[arg(long, default_value = "sharegpt")]
        format: String,
    },
}

pub async fn handle(args: DatasetArgs) -> Result<()> {
    match args.command {
        DatasetCommands::Collect { project, from, only_approved, out, format } => {
            handle_collect(project, &from, only_approved, out, &format).await
        }
    }
}

async fn handle_collect(
    project: Option<String>,
    from: &str,
    only_approved: bool,
    out: PathBuf,
    format: &str,
) -> Result<()> {
    if from != "chat" && from != "missions" && from != "all" {
        anyhow::bail!("Invalid --from '{}', expected chat|missions|all", from);
    }
    if format != "sharegpt" && format != "alpaca" {
        anyhow::bail!("Invalid --format '{}', expected sharegpt|alpaca", format);
    }
    // Collecting reads history/traces — read-only, plan-mode safe.
    let proj = projects::resolve_project(project)?;
    let system = "You are a helpful coding assistant. Answer based on the project context.".to_string();
    let mut samples = Vec::new();

    if from == "chat" || from == "all" {
        let stored = projects::find_project(&proj.id)?
            .or_else(|| proj.folder_path.as_ref().and_then(|fp| projects::find_project(fp).ok().flatten()));
        let messages = stored.map(|p| p.messages).unwrap_or_default();
        // Approval = verifier-clean assistant turn (balanced, lexical).
        let files = core_fs::list_project_files(&proj).unwrap_or_default();
        let kept = core_dataset::collect_chat_samples(&messages, &system, only_approved, &|assistant| {
            verifier::verify_response(assistant, &proj, &files, "balanced").invented_count == 0
        });
        println!("  chat: {} turn(s) → {} sample(s){}", messages.len(), kept.len(), if only_approved { " (verifier-clean only)" } else { "" });
        samples.extend(kept);
    }

    if from == "missions" || from == "all" {
        let missions = core_missions::read_missions().unwrap_or_default();
        let mut mission_samples = Vec::new();
        let mut n_traces = 0;
        for m in missions.iter().filter(|m| m.project_id == proj.id) {
            let entries = core_dataset::read_trace_entries(&m.trace_path);
            if entries.is_empty() {
                continue;
            }
            n_traces += 1;
            mission_samples.extend(core_dataset::collect_mission_samples(&entries, &system, only_approved));
        }
        println!("  missions: {} trace(s) → {} sample(s){}", n_traces, mission_samples.len(), if only_approved { " (applied edits only)" } else { "" });
        samples.extend(mission_samples);
    }

    if samples.is_empty() {
        println!("{}", style("No samples collected — chat some more or run missions first").dim());
        return Ok(());
    }
    let n = core_dataset::append_jsonl(&out, &samples, format)?;
    println!("{} collected {} sample(s) → {} (format: {})", style("✓").green(), n, out.display(), format);
    println!("  Next: local-ai finetune train --dataset {} --dry-run", out.display());
    Ok(())
}
