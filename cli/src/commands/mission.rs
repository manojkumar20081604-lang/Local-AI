//! `local-ai mission` — persisted missions, the command-center feel (Phase 4.1).
//!
//! ```text
//! MISSION #024 — Build authentication — ████████░░ 80%
//! ✓ Planner ✓ Researcher ✓ Coder ✓ Tester → Security Reviewer
//! Files: 7 modified | Tests: 31/31 | Current: security review
//! ```
//!
//! ```bash
//! local-ai mission create "add stripe" --project MyApp
//! local-ai mission list --project MyApp
//! local-ai mission show 24 --watch
//! local-ai mission resume 24 --yes
//! local-ai mission cancel 24
//! ```
//!
//! A mission is a persisted Phase-2 plan graph + step states. `resume`
//! executes the remaining steps through the same executor as `agent run`
//! and persists after every step, so a killed run resumes to completion.

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::commands::runner;
use crate::core::config::{load_config, ProviderKind};
use crate::core::{
    agents as core_agents, fs as core_fs, intelligence, missions as core_missions,
    plan as core_plan, projects, provider,
};

#[derive(Parser)]
pub struct MissionArgs {
    #[command(subcommand)]
    pub command: MissionCommands,
}

#[derive(Subcommand)]
pub enum MissionCommands {
    /// Plan a goal and persist it as a mission (read-only, no execution)
    Create {
        /// Goal, e.g. "add stripe checkout"
        goal: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        test_cmd: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "20")]
        max_steps: u32,
        #[arg(long, default_value = "50")]
        max_tool_calls: u32,
        #[arg(long, default_value = "600")]
        max_wall_secs: u64,
        #[arg(long)]
        approve: Vec<String>,
    },
    /// List missions (optionally filtered by project)
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// Show mission progress (TASKS view). `--watch` tails the trace live.
    Show {
        /// Mission number (`24`, `#024`) or id
        id: String,
        #[arg(long)]
        project: Option<String>,
        /// Stream step updates until the mission reaches a terminal state
        #[arg(long)]
        watch: bool,
        /// Max seconds to watch (default 120)
        #[arg(long, default_value = "120")]
        watch_secs: u64,
    },
    /// Execute remaining steps (same engine as `agent run`; persists per step)
    Resume {
        /// Mission number or id
        id: String,
        #[arg(long)]
        project: Option<String>,
        /// Skip per-step approval prompts
        #[arg(long)]
        yes: bool,
        /// Cap steps for this invocation (partial run; resume again later)
        #[arg(long)]
        max_steps: Option<u32>,
        #[arg(long)]
        approve: Vec<String>,
    },
    /// Cancel a mission (terminal; never executes)
    Cancel {
        /// Mission number or id
        id: String,
    },
}

pub async fn handle(
    args: MissionArgs,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    match args.command {
        MissionCommands::Create { goal, project, test_cmd, model, max_steps, max_tool_calls, max_wall_secs, approve } => {
            handle_create(goal, project, test_cmd, model, max_steps, max_tool_calls, max_wall_secs, approve).await
        }
        MissionCommands::List { project } => handle_list(project).await,
        MissionCommands::Show { id, project, watch, watch_secs } => handle_show(&id, project, watch, watch_secs).await,
        MissionCommands::Resume { id, project, yes, max_steps, approve } => {
            handle_resume(&id, project, yes, max_steps, approve, global_provider, global_url, global_lm_url).await
        }
        MissionCommands::Cancel { id } => handle_cancel(&id).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_create(
    goal: String,
    project: Option<String>,
    test_cmd: Option<String>,
    model: Option<String>,
    max_steps: u32,
    max_tool_calls: u32,
    max_wall_secs: u64,
    approve: Vec<String>,
) -> Result<()> {
    // Planning never writes to the project — plan-mode safe.
    let proj = projects::resolve_project(project)?;
    let files = core_fs::list_project_files(&proj)?;
    let intent = intelligence::detect_intent(&goal);
    let relevant = intelligence::rank_relevant_files(&goal, &files, &intent);
    let relevant: Vec<_> = relevant.into_iter().take(8).collect();
    let graph = core_plan::build_plan(&goal, &proj, &files, &intent.intent, &relevant);
    core_plan::validate_graph(&graph)?;

    let budget = core_agents::Budget {
        max_steps: max_steps.clamp(1, 100),
        max_tool_calls: max_tool_calls.clamp(1, 500),
        max_wall_secs: max_wall_secs.clamp(10, 3600),
        approve_dangerous: approve.iter().any(|a| a.to_lowercase() == "dangerous"),
    };
    let mission = core_missions::create_mission_record(
        &proj,
        &goal,
        &intent.intent.to_string(),
        graph,
        budget,
        approve.iter().any(|a| a.to_lowercase() == "dangerous"),
        test_cmd,
        model,
    )?;
    core_agents::append_trace(
        &mission.trace_path,
        &core_agents::TraceEntry::new(&mission.id, "planner", "mission_create", &goal, &format!("{} steps planned", mission.plan.steps.len()), "done"),
    )?;
    print_mission_header(&mission);
    println!("  {} steps planned — run them with: local-ai mission resume {}", style("→").dim(), mission.number);
    Ok(())
}

async fn handle_list(project: Option<String>) -> Result<()> {
    let missions = core_missions::read_missions()?;
    let proj_filter = match project {
        Some(p) => Some(projects::resolve_project(Some(p))?),
        None => None,
    };
    let mut shown = 0;
    for m in &missions {
        if let Some(pf) = &proj_filter {
            if pf.id != m.project_id {
                continue;
            }
        }
        println!(
            "#{:>3} {} {} {} — {}",
            m.number,
            core_missions::progress_bar(m.progress()),
            status_icon(m.status),
            style(&m.goal).bold(),
            style(format!("{} · {} · {}", m.status, m.project_name, m.updated_at)).dim(),
        );
        shown += 1;
    }
    if shown == 0 {
        println!("{}", style("No missions yet — create one: local-ai mission create \"<goal>\" --project MyApp").dim());
    }
    Ok(())
}

fn status_icon(s: core_missions::MissionStatus) -> String {
    match s {
        core_missions::MissionStatus::Done => style("✓").green().to_string(),
        core_missions::MissionStatus::Running => style("▶").cyan().to_string(),
        core_missions::MissionStatus::Failed => style("✗").red().to_string(),
        core_missions::MissionStatus::Cancelled => style("■").dim().to_string(),
        core_missions::MissionStatus::Pending => style("○").dim().to_string(),
    }
}

fn print_mission_header(m: &core_missions::Mission) {
    println!(
        "{}",
        style(format!("MISSION #{} — {} — {} {}", m.number, m.goal, core_missions::progress_bar(m.progress()), m.status)).bold()
    );
    // Role checklist from step states.
    let mut parts = Vec::new();
    for (step, st) in m.plan.steps.iter().zip(m.step_statuses.iter()) {
        let mark = match st.as_str() {
            "done" | "pass" => style("✓").green().to_string(),
            "pending" => style("○").dim().to_string(),
            "fail" | "blocked" | "refused" => style("✗").red().to_string(),
            _ => style("→").yellow().to_string(),
        };
        parts.push(format!("{} {}", mark, step.id));
    }
    println!("  {}", parts.join(" "));
    let n_tests = m.tests.iter().filter(|t| t.get("result").and_then(|r| r.as_str()) == Some("pass")).count();
    println!(
        "  Files: {} modified | Tests: {}/{} | Current: {}",
        m.applied.len(),
        n_tests,
        m.tests.len(),
        m.current_step().unwrap_or_else(|| "—".to_string())
    );
    println!("  Trace: {}", m.trace_path.display());
}

async fn handle_show(id: &str, _project: Option<String>, watch: bool, watch_secs: u64) -> Result<()> {
    let mission = core_missions::get_mission(id)?;
    print_mission_header(&mission);
    // CHANGES view: applied files + test outcomes.
    if !mission.applied.is_empty() {
        println!("\n{}", style("CHANGES:").bold());
        for a in mission.applied.iter().take(20) {
            println!("  {} {}", style("+").green(), a);
        }
    }
    if !mission.tests.is_empty() {
        println!("\n{}", style("TESTS:").bold());
        for t in &mission.tests {
            println!(
                "  {}={}",
                t.get("step").and_then(|s| s.as_str()).unwrap_or("?"),
                t.get("result").and_then(|r| r.as_str()).unwrap_or("?")
            );
        }
    }
    // LOGS view: last trace lines.
    print_trace_tail(&mission, 10)?;
    if watch {
        watch_mission(&mission, watch_secs).await?;
    }
    Ok(())
}

fn read_trace_lines(m: &core_missions::Mission) -> Vec<String> {
    std::fs::read_to_string(&m.trace_path)
        .unwrap_or_default()
        .lines()
        .map(|s| s.to_string())
        .collect()
}

fn print_trace_tail(m: &core_missions::Mission, n: usize) -> Result<()> {
    let lines = read_trace_lines(m);
    if lines.is_empty() {
        return Ok(());
    }
    println!("\n{}", style(format!("LOGS (last {} of {}):", n.min(lines.len()), lines.len())).bold());
    for line in lines.iter().rev().take(n).rev() {
        // Compact one-line summary per trace entry.
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => println!(
                "  [{}] {}:{} — {}",
                v.get("ts").and_then(|x| x.as_str()).unwrap_or("?"),
                v.get("agent").and_then(|x| x.as_str()).unwrap_or("?"),
                v.get("action").and_then(|x| x.as_str()).unwrap_or("?"),
                v.get("status").and_then(|x| x.as_str()).unwrap_or("?")
            ),
            Err(_) => println!("  {}", line),
        }
    }
    Ok(())
}

async fn watch_mission(m: &core_missions::Mission, watch_secs: u64) -> Result<()> {
    use std::time::{Duration, Instant};
    println!("\n{} watching {} (Ctrl-C to stop)…", style("👁").dim(), m.trace_path.display());
    let start = Instant::now();
    let mut seen = read_trace_lines(m).len();
    loop {
        if start.elapsed().as_secs() >= watch_secs {
            println!("{} watch timeout ({}s)", style("→").dim(), watch_secs);
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        // Re-read mission status (another process/run may have advanced it).
        let current = core_missions::get_mission(&m.id).unwrap_or_else(|_| m.clone());
        let lines = read_trace_lines(&current);
        for line in lines.iter().skip(seen) {
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => println!(
                    "  {} {}:{} — {}",
                    style("+").cyan(),
                    v.get("agent").and_then(|x| x.as_str()).unwrap_or("?"),
                    v.get("action").and_then(|x| x.as_str()).unwrap_or("?"),
                    v.get("status").and_then(|x| x.as_str()).unwrap_or("?")
                ),
                Err(_) => println!("  {}", line),
            }
        }
        seen = lines.len();
        if current.status.is_terminal() {
            println!("{} mission #{} is now {}", style("✓").green(), current.number, current.status);
            break;
        }
    }
    Ok(())
}

async fn handle_cancel(id: &str) -> Result<()> {
    let mut mission = core_missions::get_mission(id)?;
    if mission.status.is_terminal() {
        println!("Mission #{} is already {}", mission.number, mission.status);
        return Ok(());
    }
    mission.status = core_missions::MissionStatus::Cancelled;
    core_missions::save_mission(&mission)?;
    core_agents::append_trace(
        &mission.trace_path,
        &core_agents::TraceEntry::new(&mission.id, "orchestrator", "cancel", &mission.goal, "cancelled by user", "cancelled"),
    )?;
    println!("{} mission #{} cancelled", style("■").dim(), mission.number);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_resume(
    id: &str,
    project: Option<String>,
    yes: bool,
    max_steps: Option<u32>,
    approve: Vec<String>,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    let mut mission = core_missions::get_mission(id)?;
    if mission.status.is_terminal() {
        anyhow::bail!("Mission #{} is {} — nothing to resume", mission.number, mission.status);
    }
    crate::core::config::require_build_mode("mission resume")?;

    // Resolve the project: explicit override, else the folder stored at creation.
    let proj = match project {
        Some(p) => projects::resolve_project(Some(p))?,
        None => match &mission.folder_path {
            Some(folder) => projects::resolve_project(Some(folder.clone()))?,
            None => projects::resolve_project(None)?,
        },
    };
    let files = core_fs::list_project_files(&proj)?;
    let relevant_paths = runner::relevant_paths_for(&mission.goal, &files);

    let cfg = load_config().unwrap_or_default();
    let provider_kind = global_provider.unwrap_or_else(|| cfg.provider.active.clone());
    let provider_url = crate::core::config::resolve_provider_url(&provider_kind, &cfg, global_url.as_deref(), global_lm_url.as_deref());

    // Model: stored override, else first available (offline → manual coder).
    let model: Option<String> = if let Some(m) = mission.model.clone() {
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
        eprintln!("{} no model reachable — coder steps will be MANUAL", style("!").yellow());
    }

    let approve_dangerous = if approve.is_empty() {
        mission.approve_dangerous
    } else {
        approve.iter().any(|a| a.to_lowercase() == "dangerous")
    };
    let mut budget = mission.budget.clone();
    budget.approve_dangerous = approve_dangerous;

    mission.status = core_missions::MissionStatus::Running;
    core_missions::save_mission(&mission)?;
    print_mission_header(&mission);

    // Rebuild runner state from the persisted mission.
    let mut state = runner::RunState {
        step_statuses: mission
            .plan
            .steps
            .iter()
            .zip(mission.step_statuses.iter())
            .map(|(s, st)| serde_json::json!({"id": s.id, "status": st}))
            .collect(),
        applied: mission.applied.clone(),
        skipped: mission.skipped.iter().map(|e| (e.path.clone(), e.reason.clone())).collect(),
        tests: mission.tests.clone(),
    };
    let ctx = runner::ExecCtx {
        goal: mission.goal.clone(),
        test_cmd: mission.test_cmd.clone(),
        model: model.clone(),
        provider_kind: provider_kind.clone(),
        provider_url: provider_url.clone(),
        yes,
        budget: budget.clone(),
        max_steps_this_run: max_steps,
    };
    let mut orch = core_agents::Orchestrator::new(
        mission.id.clone(),
        budget.clone(),
        if yes { core_agents::ApprovalGate::Auto } else { core_agents::ApprovalGate::Always },
    );
    // Persist after every step (kill-safe).
    let hook = |step_id: &str, status: &str| {
        if let Ok(mut m) = core_missions::get_mission(&mission.id) {
            let _ = core_missions::record_step_status(&mut m, step_id, status);
        }
    };
    let outcome = runner::run_graph(
        &proj,
        &files,
        &relevant_paths,
        &mission.plan,
        &ctx,
        &cfg,
        &mut state,
        &mut orch,
        &mission.trace_path,
        &mission.id,
        Some(&hook),
    )
    .await?;

    // Sync runner state back into the mission record.
    let mut reloaded = core_missions::get_mission(&mission.id)?;
    for entry in &state.step_statuses {
        if let (Some(id), Some(st)) = (
            entry.get("id").and_then(|v| v.as_str()),
            entry.get("status").and_then(|v| v.as_str()),
        ) {
            let _ = core_missions::record_step_status(&mut reloaded, id, st);
        }
    }
    reloaded.applied = state.applied.clone();
    reloaded.skipped = state
        .skipped
        .iter()
        .map(|(path, reason)| core_missions::SkippedEntry { path: path.clone(), reason: reason.clone() })
        .collect();
    reloaded.tests = state.tests.clone();
    reloaded.tests_green = outcome.tests_green;
    reloaded.model = model;
    let all_terminal = reloaded
        .step_statuses
        .iter()
        .all(|s| s != "pending");
    reloaded.status = if all_terminal {
        core_missions::MissionStatus::Done
    } else if outcome.stop != runner::StopReason::Finished {
        // Stopped early (cap/budget) — stays resumable.
        core_missions::MissionStatus::Running
    } else {
        core_missions::MissionStatus::Running
    };
    if outcome.stop == runner::StopReason::Finished && !all_terminal {
        reloaded.status = core_missions::MissionStatus::Running;
    }
    core_missions::save_mission(&reloaded)?;

    // Final report next to the trace.
    let report = serde_json::json!({
        "mission_id": reloaded.id,
        "number": reloaded.number,
        "goal": reloaded.goal,
        "status": reloaded.status.to_string(),
        "stop": format!("{:?}", outcome.stop),
        "steps": state.step_statuses,
        "tests": state.tests,
        "tests_green": outcome.tests_green,
        "applied": state.applied,
        "trace": reloaded.trace_path.to_string_lossy(),
    });
    if let Some(parent) = reloaded.trace_path.parent() {
        let _ = std::fs::write(parent.join("report.json"), serde_json::to_string_pretty(&report).unwrap_or_default());
    }
    core_agents::append_trace(
        &reloaded.trace_path,
        &core_agents::TraceEntry::new(&reloaded.id, "orchestrator", "resume_finish", &reloaded.goal, &serde_json::to_string(&report).unwrap_or_default(), "done"),
    )?;

    println!("\n{}", style(format!("MISSION #{} — {}", reloaded.number, reloaded.status)).bold());
    println!("  Steps this run: {} | Tool calls: {} | Tests: {}",
        outcome.steps_this_run,
        outcome.tool_calls,
        state.tests.iter().map(|t| format!("{}={}", t.get("step").and_then(|s| s.as_str()).unwrap_or("?"), t.get("result").and_then(|r| r.as_str()).unwrap_or("?"))).collect::<Vec<_>>().join(", "));
    println!("  Trace: {}", reloaded.trace_path.display());
    match outcome.stop {
        runner::StopReason::Finished if all_terminal => println!("{} mission complete — green", style("✅").green().bold()),
        runner::StopReason::StepCapReached => println!("{} step cap reached — resume again: local-ai mission resume {}", style("→").dim(), reloaded.number),
        runner::StopReason::BudgetExceeded(ref e) => println!("{} budget exceeded ({}) — resume with higher budget", style("!").yellow(), e),
        _ => println!("{} mission paused — resume with: local-ai mission resume {}", style("→").dim(), reloaded.number),
    }
    Ok(())
}
