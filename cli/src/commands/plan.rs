//! `local-ai plan` + `local-ai exec-plan` — Plan Mode 2.0 (Phase 1.4).
//!
//! `plan` builds a deterministic execution graph (no writes, plan-mode safe).
//! `exec-plan` previews (`--dry-run`, plan-mode safe) or executes it step by
//! step: inspect/context/test steps run with an approval gate, edit/review
//! steps print as MANUAL until the Phase 2 Coder lands.

use anyhow::Result;
use clap::Parser;
use console::style;
use std::path::PathBuf;

use crate::core::{fs as core_fs, intelligence, plan as core_plan, projects};

#[derive(Parser)]
pub struct PlanArgs {
    /// Goal, e.g. "add JWT authentication"
    pub goal: String,

    /// Project id/name/path (default: current dir)
    #[arg(long)]
    pub project: Option<String>,

    /// Print machine-readable plan-graph JSON instead of the human view
    #[arg(long)]
    pub json: bool,

    /// Skip persisting the plan to the cache dir
    #[arg(long)]
    pub no_save: bool,
}

pub async fn handle_plan(args: PlanArgs) -> Result<()> {
    // No build gate — planning never writes to the project.
    let proj = projects::resolve_project(args.project)?;
    let files = core_fs::list_project_files(&proj)?;
    let intent = intelligence::detect_intent(&args.goal);
    let relevant = intelligence::rank_relevant_files(&args.goal, &files, &intent);
    let relevant: Vec<_> = relevant.into_iter().take(8).collect();

    let graph = core_plan::build_plan(&args.goal, &proj, &files, &intent.intent, &relevant);
    core_plan::validate_graph(&graph)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&graph)?);
    } else {
        print_human(&graph);
    }

    if !args.no_save {
        let path = core_plan::save_plan(&proj, &graph)?;
        eprintln!(
            "\n{} saved to {}",
            style("Plan").dim(),
            style(path.display()).dim()
        );
        if !args.json {
            eprintln!(
                "{} Approve ✓ / Modify ↻ / Reject ✗ / Execute ▶: local-ai exec-plan {} [--dry-run]",
                style("→").dim(),
                path.display()
            );
        }
    }
    Ok(())
}

fn print_human(graph: &core_plan::PlanGraph) {
    println!("{}", style(format!("PLAN — {}", graph.goal)).bold());
    println!(
        "{} Project: {} (intent: {}, {})",
        style("Goal:").dim(),
        graph.project_name,
        graph.intent,
        style(format!("{} steps", graph.steps.len())).dim()
    );
    let mut prev: Option<&str> = None;
    for (i, s) in graph.steps.iter().enumerate() {
        let arrow = if prev.map(|p| s.depends_on.iter().any(|d| d == p)).unwrap_or(false) {
            "  ↓"
        } else {
            ""
        };
        if !arrow.is_empty() {
            println!("{}", style(arrow).dim());
        }
        let kind_tag = match s.kind {
            core_plan::StepKind::Edit
            | core_plan::StepKind::Create
            | core_plan::StepKind::Manual
            | core_plan::StepKind::Review => style("MANUAL").yellow().to_string(),
            _ => style("auto").green().to_string(),
        };
        println!(
            "[{}] {} [{}] ({})",
            style(i + 1).cyan().bold(),
            style(&s.title).bold(),
            s.kind,
            kind_tag
        );
        if !s.files.is_empty() {
            let shown: Vec<_> = s.files.iter().take(6).cloned().collect();
            println!("      {} {}", style("files:").dim(), shown.join(", "));
            if s.files.len() > 6 {
                println!("      {} +{} more", style("…").dim(), s.files.len() - 6);
            }
        }
        if let Some(cmd) = &s.cmd {
            println!("      {} {}", style("cmd:").dim(), style(cmd).cyan());
        } else if s.kind == core_plan::StepKind::Test {
            println!("      {} (none detected — verify manually)", style("cmd:").dim());
        }
        if !s.notes.is_empty() {
            println!("      {} {}", style("§").dim(), s.notes);
        }
        prev = Some(&s.id);
    }
}

// ---------------------------------------------------------------------------
// exec-plan
// ---------------------------------------------------------------------------

#[derive(Parser)]
pub struct ExecPlanArgs {
    /// Path to a plan JSON file (from `local-ai plan`)
    pub plan: PathBuf,

    /// Project override (default: resolve like the plan's project)
    #[arg(long)]
    pub project: Option<String>,

    /// Preview every step without executing anything (plan-mode safe)
    #[arg(long)]
    pub dry_run: bool,

    /// Skip per-step approval prompts
    #[arg(long)]
    pub yes: bool,

    /// Only run steps with these ids (repeatable); dependencies still checked
    #[arg(long)]
    pub only: Vec<String>,
}

pub async fn handle_exec_plan(args: ExecPlanArgs) -> Result<()> {
    // Real execution runs commands → build gate. Dry-run is read-only.
    if !args.dry_run {
        crate::core::config::require_build_mode("exec-plan")?;
    }
    let graph = core_plan::load_plan(&args.plan)?;
    let proj = match args.project {
        Some(p) => projects::resolve_project(Some(p))?,
        None => projects::resolve_project(None).unwrap_or_else(|_| projects::Project {
            id: graph.project_id.clone(),
            name: graph.project_name.clone(),
            created_at: graph.created_at.clone(),
            updated_at: graph.created_at.clone(),
            folder_path: None,
            messages: Vec::new(),
        }),
    };

    println!("{}", style(format!("EXEC-PLAN — {}", graph.goal)).bold());
    if args.dry_run {
        println!("{} dry-run: nothing will execute", style("[preview]").dim());
    }

    let mut results: Vec<serde_json::Value> = Vec::new();
    for step in &graph.steps {
        if !args.only.is_empty() && !args.only.contains(&step.id) {
            results.push(serde_json::json!({"id": step.id, "status": "skipped"}));
            continue;
        }
        let header = format!("[{}] {} ({})", step.id, step.title, step.kind);
        if args.dry_run {
            println!("\n{} {}", style("would-run").dim(), header);
            describe_step(step);
            results.push(serde_json::json!({"id": step.id, "status": "preview"}));
            continue;
        }
        println!("\n{} {}", style("▶").cyan().bold(), style(&header).bold());
        describe_step(step);
        let status = execute_step(&proj, step, args.yes)?;
        results.push(serde_json::json!({"id": step.id, "status": status}));
    }

    let report = serde_json::json!({
        "goal": graph.goal,
        "intent": graph.intent,
        "plan_version": graph.version,
        "dry_run": args.dry_run,
        "steps": results,
    });
    if !args.dry_run {
        // Best-effort run report next to the plan (feeds future missions).
        match core_plan::save_run_report(&proj, &args.plan, &report) {
            Ok(p) => eprintln!("\n{} run report: {}", style("✓").green(), p.display()),
            Err(e) => eprintln!("\n{} could not save run report: {}", style("!").yellow(), e),
        }
    }
    Ok(())
}

fn describe_step(step: &core_plan::PlanStep) {
    use core_plan::StepKind as K;
    match step.kind {
        K::Inspect => println!("  {} list project files", style("auto:").green()),
        K::Context => {
            if step.files.is_empty() {
                println!("  {} no relevant files ranked", style("auto:").green());
            } else {
                println!("  {} read {} file(s): {}", style("auto:").green(), step.files.len(), step.files.join(", "));
            }
        }
        K::Test => match &step.cmd {
            Some(c) => println!("  {} run `{}` (approval-gated)", style("auto:").green(), c),
            None => println!("  {} no command detected — verify manually", style("manual:").yellow()),
        },
        K::Edit | K::Create | K::Review | K::Manual => {
            println!("  {} {}", style("MANUAL:").yellow(), step.notes);
            if !step.files.is_empty() {
                println!("  {} {}", style("files:").dim(), step.files.join(", "));
            }
        }
    }
}

fn confirm_or_yes(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    match dialoguer::Confirm::new().with_prompt(prompt).default(false).interact_opt()? {
        Some(v) => Ok(v),
        None => {
            eprintln!("{} non-interactive — re-run with --yes to approve", style("!").yellow());
            Ok(false)
        }
    }
}

/// Execute one auto step. Returns a status string for the run report.
fn execute_step(
    proj: &projects::Project,
    step: &core_plan::PlanStep,
    yes: bool,
) -> Result<String> {
    use core_plan::StepKind as K;
    match step.kind {
        K::Inspect => {
            let files = core_fs::list_project_files(proj)?;
            let dirs = files.iter().filter(|f| f.is_directory).count();
            let docs = files.len() - dirs;
            println!("  {} {} files ({} dirs)", style("✓").green(), docs, dirs);
            Ok("done".to_string())
        }
        K::Context => {
            if step.files.is_empty() {
                println!("  {} nothing to load", style("→").dim());
                return Ok("done".to_string());
            }
            let mut loaded = 0usize;
            let mut missing = Vec::new();
            for f in &step.files {
                match core_fs::read_project_file(proj, f) {
                    Ok(c) => {
                        loaded += 1;
                        println!("  {} {} ({} chars)", style("✓").green(), f, c.len());
                    }
                    Err(_) => missing.push(f.clone()),
                }
            }
            for m in &missing {
                println!("  {} {} (unreadable — skipped)", style("!").yellow(), m);
            }
            println!("  {} loaded {}/{} files", style("→").dim(), loaded, step.files.len());
            Ok(if missing.is_empty() { "done".to_string() } else { "partial".to_string() })
        }
        K::Test => {
            let cmd = match &step.cmd {
                Some(c) => c.clone(),
                None => {
                    println!("  {} no command — mark verified manually", style("→").dim());
                    return Ok("manual".to_string());
                }
            };
            if !confirm_or_yes(&format!("Run `{}`?", cmd), yes)? {
                println!("  {} rejected by user", style("✗").red());
                return Ok("rejected".to_string());
            }
            let result = core_fs::run_project_command(proj, &cmd)?;
            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
            }
            if !result.stderr.is_empty() {
                eprint!("{}", result.stderr);
            }
            if result.success {
                println!("  {} exit {}", style("✓ PASS").green().bold(), result.exit_code.unwrap_or(0));
                Ok("pass".to_string())
            } else {
                println!("  {} exit {}", style("✗ FAIL").red().bold(), result.exit_code.unwrap_or(1));
                Ok("fail".to_string())
            }
        }
        K::Edit | K::Create | K::Review | K::Manual => {
            println!("  {} left for human/agent — see notes above", style("→").dim());
            Ok("manual".to_string())
        }
    }
}
