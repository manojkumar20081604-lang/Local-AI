//! `local-ai propose` — self-improvement proposals as plan-graphs (Phase 5.3).
//!
//! ```bash
//! local-ai propose --project MyApp
//! local-ai propose --project MyApp --json
//! local-ai propose --project MyApp --apply rebuild-index-symbols --yes
//! ```
//!
//! Proposals are deterministic rules over `metrics` (retrieval hit-rate,
//! verifier reject-rate, test pass-rate). They are **never silent
//! self-modification**: `--apply` only saves the proposal's plan-graph to
//! the plans cache and prints the `exec-plan` command. Read-only otherwise
//! (plan-mode safe).

use anyhow::Result;
use clap::Parser;
use console::style;

use crate::core::{fs as core_fs, metrics as core_metrics, projects};

#[derive(Parser)]
pub struct ProposeArgs {
    /// Project id/name/path (default: current dir)
    #[arg(long)]
    pub project: Option<String>,

    /// Print machine-readable JSON instead of the human view
    #[arg(long)]
    pub json: bool,

    /// Apply a proposal by id (saves its plan-graph, prints next command)
    #[arg(long)]
    pub apply: Option<String>,

    /// Skip the confirmation prompt for --apply
    #[arg(long)]
    pub yes: bool,
}

pub async fn handle(args: ProposeArgs) -> Result<()> {
    // Read-only by default; --apply only writes to the plans cache (like `plan`).
    let proj = projects::resolve_project(args.project.clone())?;
    let metrics = core_metrics::collect_project_metrics(&proj)?;
    let files = core_fs::list_project_files(&proj).unwrap_or_default();
    let proposals = core_metrics::build_proposals(&metrics, &proj, &files);

    if let Some(id) = args.apply {
        let p = proposals.iter().find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("Unknown proposal '{}' — available: {}", id, proposals.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")))?;
        if !args.yes {
            let ok = dialoguer::Confirm::new()
                .with_prompt(format!("Save plan for '{}'?", p.title))
                .default(false)
                .interact_opt()?;
            if !ok.unwrap_or(false) {
                println!("{} rejected — nothing saved", style("✗").red());
                return Ok(());
            }
        }
        let path = core_metrics::save_proposal_plan(&proj, p)?;
        println!("{} proposal '{}' → plan saved to {}", style("✓").green(), p.id, path.display());
        println!("  {} Inspect: local-ai exec-plan {} --dry-run --project {}", style("→").dim(), path.display(), proj.name);
        println!("  {} Apply = run the plan above (edits stay approval-gated); Reject = do nothing", style("→").dim());
        return Ok(());
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&proposals)?);
        return Ok(());
    }

    println!("{}", style(format!("PROPOSE — {} ({} proposal(s))", proj.name, proposals.len())).bold());
    for p in &proposals {
        let sev = match p.severity.as_str() {
            "high" => style("HIGH").red().bold().to_string(),
            "medium" => style("MED").yellow().to_string(),
            _ => style(p.severity.to_uppercase()).dim().to_string(),
        };
        println!("\n  [{}] {} {}", style(&p.id).cyan().bold(), p.title, sev);
        println!("    {} {}", style("why:").dim(), p.rationale);
        println!("    {} {}", style("gain:").dim(), p.expected_gain);
        println!("    {} {}", style("inspect:").dim(), p.inspect.join(" · "));
        println!("    {} plan: {} steps ({})", style("plan:").dim(), p.plan.steps.len(),
            p.plan.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(" → "));
        println!("    {} [Inspect] local-ai exec-plan <saved> --dry-run  [Apply] --apply {} --yes  [Reject] do nothing",
            style("→").dim(), p.id);
    }
    println!("\n  {} proposals are plan-graphs — never silent self-modification", style("→").dim());
    println!("  {} local-ai propose --project {} --apply <id> --yes", style("→").dim(), proj.name);
    Ok(())
}
