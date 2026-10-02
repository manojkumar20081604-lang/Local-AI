//! `local-ai agent run` — orchestrated specialists (Phase 2).
//!
//! ```text
//! User → Planner → {Researcher, Coder, Tester, Debugger, Reviewer} → Verifier → User
//! ```
//!
//! Foreground, bounded, user-invoked (no daemon). Every agent I/O appends to
//! `~/.cache/local-ai/<project-id>/missions/<ts>/trace.jsonl`.
//!
//! ```bash
//! local-ai agent run "add auth" --project MyApp --max-steps 20 --approve dangerous
//! local-ai agent run "fix failing tests" --project MyApp --dry-run   # preview only
//! local-ai agent run "fix tests" --project MyApp --yes --test-cmd "cargo test"
//! ```
//!
//! Step execution lives in [`super::runner`] so `mission resume` (Phase 4)
//! runs the identical loop against a persisted mission.

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::commands::runner;
use crate::core::config::{load_config, ProviderKind};
use crate::core::{
    agents as core_agents, fs as core_fs, intelligence, plan as core_plan, projects, provider,
};

#[derive(Parser)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommands,
}

#[derive(Subcommand)]
pub enum AgentCommands {
    /// Run the orchestrated specialists for a goal (planner → coder → tester → reviewer)
    Run {
        /// Goal, e.g. "add JWT authentication" or "fix failing tests"
        goal: String,

        /// Project id/name/path (default: current dir)
        #[arg(long)]
        project: Option<String>,

        /// Max plan steps / agent turns (default 20)
        #[arg(long, default_value = "20")]
        max_steps: u32,

        /// Max tool-equivalent calls (file reads + execs + LLM calls, default 50)
        #[arg(long, default_value = "50")]
        max_tool_calls: u32,

        /// Max wall-clock seconds for the whole run (default 600)
        #[arg(long, default_value = "600")]
        max_wall_secs: u64,

        /// Test command override (default: auto-detected from Cargo.toml/package.json/…)
        #[arg(long)]
        test_cmd: Option<String>,

        /// Model id (default: first available from the provider; offline → manual coder)
        #[arg(long)]
        model: Option<String>,

        /// Provider override (default: config/auto)
        #[arg(long)]
        provider: Option<String>,

        /// Preview the mission without executing anything (plan-mode safe)
        #[arg(long)]
        dry_run: bool,

        /// Skip per-step approval prompts (dangerous commands still need --approve dangerous)
        #[arg(long)]
        yes: bool,

        /// Approvals to grant, e.g. `--approve dangerous` to allow rm -rf/mkfs/exfil
        #[arg(long)]
        approve: Vec<String>,
    },
}

pub async fn handle(
    args: AgentArgs,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    match args.command {
        AgentCommands::Run {
            goal,
            project,
            max_steps,
            max_tool_calls,
            max_wall_secs,
            test_cmd,
            model,
            provider,
            dry_run,
            yes,
            approve,
        } => {
            handle_run(
                goal,
                project,
                max_steps,
                max_tool_calls,
                max_wall_secs,
                test_cmd,
                model,
                provider,
                dry_run,
                yes,
                approve,
                global_provider,
                global_url,
                global_lm_url,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_run(
    goal: String,
    project: Option<String>,
    max_steps: u32,
    max_tool_calls: u32,
    max_wall_secs: u64,
    test_cmd_override: Option<String>,
    model_override: Option<String>,
    provider_override: Option<String>,
    dry_run: bool,
    yes: bool,
    approve: Vec<String>,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    let cfg = load_config().unwrap_or_default();
    let provider_kind = if let Some(p) = &provider_override {
        p.parse().unwrap_or_else(|e| {
            eprintln!("{}", e);
            std::process::exit(1)
        })
    } else {
        global_provider.clone().unwrap_or_else(|| cfg.provider.active.clone())
    };
    let provider_url = crate::core::config::resolve_provider_url(
        &provider_kind,
        &cfg,
        global_url.as_deref(),
        global_lm_url.as_deref(),
    );
    let approve_dangerous = approve.iter().any(|a| a.to_lowercase() == "dangerous");
    let budget = core_agents::Budget {
        max_steps: max_steps.clamp(1, 100),
        max_tool_calls: max_tool_calls.clamp(1, 500),
        max_wall_secs: max_wall_secs.clamp(10, 3600),
        approve_dangerous,
    };
    let approval = if yes {
        core_agents::ApprovalGate::Auto
    } else {
        core_agents::ApprovalGate::Always
    };

    // --- Planner (deterministic, read-only) ---
    let proj = projects::resolve_project(project)?;
    let files = core_fs::list_project_files(&proj)?;
    let intent = intelligence::detect_intent(&goal);
    let relevant = intelligence::rank_relevant_files(&goal, &files, &intent);
    let relevant: Vec<_> = relevant.into_iter().take(8).collect();
    let relevant_paths: Vec<String> = relevant.iter().map(|r| r.file.path.clone()).collect();
    let graph = core_plan::build_plan(&goal, &proj, &files, &intent.intent, &relevant);
    core_plan::validate_graph(&graph)?;

    let (mission_id, trace_path) = core_agents::create_mission(&proj)?;
    let mut orch = core_agents::Orchestrator::new(mission_id.clone(), budget.clone(), approval);
    core_agents::append_trace(
        &trace_path,
        &core_agents::TraceEntry::new(
            &mission_id,
            "planner",
            "build_plan",
            &goal,
            &serde_json::to_string(&graph)?,
            "done",
        ),
    )?;

    println!("{}", style(format!("MISSION {} — {}", mission_id, goal)).bold());
    println!(
        "{} project: {} (intent: {}, {} steps, budget: {} steps/{} tools/{}s){}",
        style("→").dim(),
        proj.name,
        intent.intent,
        graph.steps.len(),
        budget.max_steps,
        budget.max_tool_calls,
        budget.max_wall_secs,
        if approve_dangerous {
            style(" [dangerous approved]").red().to_string()
        } else {
            String::new()
        }
    );
    println!("{} trace: {}", style("→").dim(), trace_path.display());

    if dry_run || crate::core::config::is_plan_mode() {
        println!("\n{} mission preview (nothing executes):", style("[dry-run]").dim());
        for (i, s) in graph.steps.iter().enumerate() {
            println!(
                "  [{}] {} ({}){}",
                s.id,
                s.title,
                s.kind,
                if s.cmd.is_some() {
                    format!(" — `{}`", s.cmd.as_deref().unwrap_or(""))
                } else {
                    String::new()
                }
            );
            let _ = i;
        }
        core_agents::append_trace(
            &trace_path,
            &core_agents::TraceEntry::new(&mission_id, "orchestrator", "dry_run", &goal, "preview only", "preview"),
        )?;
        if crate::core::config::is_plan_mode() && !dry_run {
            println!("  {} plan mode is active — switch to build to execute", style("→").dim());
        }
        return Ok(());
    }
    crate::core::config::require_build_mode("agent run")?;

    // --- Model resolution (non-fatal: offline → manual coder, tester still runs) ---
    let model: Option<String> = if let Some(m) = model_override {
        Some(m)
    } else {
        match provider::list_models_unified(&provider_kind, &provider_url, &cfg).await {
            Ok(v) if !v.is_empty() => Some(v[0].id.clone()),
            _ => None,
        }
    };
    if let Some(m) = &model {
        println!("{} model: {} (provider: {})", style("agent:").dim(), m, provider_kind);
    } else {
        eprintln!(
            "{} no model reachable — coder steps will be MANUAL, tester/reviewer still run",
            style("!").yellow()
        );
    }
    core_agents::append_trace(
        &trace_path,
        &core_agents::TraceEntry::new(
            &mission_id,
            "orchestrator",
            "model_resolve",
            &provider_kind.to_string(),
            model.as_deref().unwrap_or("none (offline manual coder)"),
            "done",
        ),
    )?;

    // --- Execute via the shared runner (identical loop as `mission resume`) ---
    let ctx = runner::ExecCtx {
        goal: goal.clone(),
        test_cmd: test_cmd_override,
        model: model.clone(),
        provider_kind: provider_kind.clone(),
        provider_url: provider_url.clone(),
        yes,
        budget: budget.clone(),
        max_steps_this_run: None,
    };
    let mut state = runner::RunState::default();
    let outcome = runner::run_graph(
        &proj,
        &files,
        &relevant_paths,
        &graph,
        &ctx,
        &cfg,
        &mut state,
        &mut orch,
        &trace_path,
        &mission_id,
        None,
    )
    .await?;
    if outcome.stop != runner::StopReason::Finished {
        eprintln!("{} stopped early: {:?}", style("!").yellow(), outcome.stop);
    }

    // --- Final report ---
    let report = serde_json::json!({
        "mission_id": mission_id,
        "goal": goal,
        "intent": intent.intent.to_string(),
        "model": model,
        "budget": budget,
        "steps": state.step_statuses,
        "tests": state.tests,
        "tests_green": outcome.tests_green,
        "applied": state.applied,
        "skipped": state.skipped,
        "trace": trace_path.to_string_lossy(),
    });
    if let Some(parent) = trace_path.parent() {
        let rp = parent.join("report.json");
        if let Ok(s) = serde_json::to_string_pretty(&report) {
            let _ = std::fs::write(&rp, s);
        }
    }
    core_agents::append_trace(
        &trace_path,
        &core_agents::TraceEntry::new(&mission_id, "orchestrator", "finish", &goal, &serde_json::to_string(&report).unwrap_or_default(), "done"),
    )?;

    println!("\n{}", style(format!("MISSION {} — done", mission_id)).bold());
    println!("  Steps: {}/{} | Tools: {}/{} | Applied: {} | Skipped: {}",
        orch.steps_used, budget.max_steps, orch.tool_calls, budget.max_tool_calls,
        state.applied.len(), state.skipped.len());
    if !state.tests.is_empty() {
        println!("  Tests: {}",
            state.tests.iter().map(|t| format!("{}={}", t.get("step").and_then(|s| s.as_str()).unwrap_or("?"), t.get("result").and_then(|r| r.as_str()).unwrap_or("?"))).collect::<Vec<_>>().join(", "));
    }
    println!("  Trace: {}", trace_path.display());
    if outcome.tests_green {
        println!("{} mission complete — green", style("✅").green().bold());
    } else {
        println!("{} mission finished with manual/failed steps — see trace", style("→").dim());
    }
    Ok(())
}
