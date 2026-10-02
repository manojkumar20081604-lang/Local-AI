//! `local-ai metrics` — project health from real traces (Phase 5.3).
//!
//! ```bash
//! local-ai metrics --project MyApp
//! local-ai metrics --project MyApp --json
//! ```
//!
//! Read-only and plan-mode safe: reads `missions.json`, `trace.jsonl`
//! files, `debug/*.json` transcripts and `index.json` — never writes to
//! the project.

use anyhow::Result;
use clap::Parser;
use console::style;

use crate::core::{fs as core_fs, metrics as core_metrics, projects};

#[derive(Parser)]
pub struct MetricsArgs {
    /// Project id/name/path (default: current dir)
    #[arg(long)]
    pub project: Option<String>,

    /// Print machine-readable JSON instead of the human table
    #[arg(long)]
    pub json: bool,
}

pub async fn handle(args: MetricsArgs) -> Result<()> {
    // No build gate — metrics never write to the project.
    let proj = projects::resolve_project(args.project)?;
    let m = core_metrics::collect_project_metrics(&proj)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    println!("{}", style(format!("METRICS — {} ({})", m.project_name, m.project_id)).bold());
    println!("  missions: {} total ({} done) | steps: {}/{} finished",
        m.missions_total, m.missions_done, m.steps_finished, m.steps_total);
    let pct = |o: Option<f32>| o.map(|r| format!("{:.0}%", r * 100.0)).unwrap_or_else(|| "n/a (no data)".to_string());
    println!("  retrieval hit-rate:   {}  (context {}/{} done, index {} files/{} symbols/{} edges{})",
        style(pct(m.retrieval_hit_rate)).bold(),
        m.context_done, m.context_total, m.index_files, m.index_symbols, m.index_edges,
        match m.index_stale {
            Some(true) => " — STALE",
            Some(false) => "",
            None => " — no index",
        });
    println!("  verifier reject-rate: {}  (review flagged {}/{} + coder refused {})",
        style(pct(m.verifier_reject_rate)).bold(),
        m.review_flagged, m.review_total, m.coder_refused);
    println!("  test pass-rate:       {}  (tests {}/{} pass, debug {}/{} pass, traces {}/{})",
        style(pct(m.test_pass_rate)).bold(),
        m.tests_pass, m.tests_total, m.debug_pass, m.debug_runs,
        m.trace_entries, m.trace_files);

    if !m.has_data() {
        println!("\n  {} no missions/traces/debug runs yet — run one mission to seed metrics", style("→").dim());
        println!("  {} local-ai mission create \"<goal>\" --project {} && local-ai mission resume <id> --yes",
            style("→").dim(), proj.name);
    } else {
        // Nudge toward the file inventory so the numbers stay grounded.
        let files = core_fs::list_project_files(&proj).unwrap_or_default();
        let n_files = files.iter().filter(|f| !f.is_directory).count();
        println!("\n  {} project files: {} | traces: {} files | next: local-ai propose --project {}",
            style("→").dim(), n_files, m.trace_files, proj.name);
    }
    Ok(())
}
