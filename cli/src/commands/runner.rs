//! Shared plan-step executor (Phase 2 engine, Phase 4 persistence).
//!
//! Both `agent run` (ad-hoc mission) and `mission resume` (persisted
//! mission) execute the deterministic Phase-1.4 plan graph through this
//! module: inspect/context auto-run, test steps are approval-gated,
//! edit/create steps go through the Coder (LLM + SEARCH-verified apply),
//! review steps run the verifier + security checklist.
//!
//! The executor is resumable: `RunState` carries step statuses across
//! invocations, already-finished steps are skipped without consuming
//! budget, and `on_step` persists state after every step (missions use
//! it; `agent run` passes `None`). `max_steps_this_run` bounds one
//! invocation — the test suite uses it to simulate a kill between runs.

use anyhow::Result;
use console::style;

use crate::core::config::{AppConfig, ProviderKind};
use crate::core::{
    agents as core_agents, debug as core_debug, edits as core_edits, fs as core_fs,
    intelligence, plan as core_plan, provider, ui_events, verifier,
};

// Output interception: every println!/eprintln! below reroutes to the TUI
// event bus while `local-ai tui` owns the screen, and prints normally
// otherwise — so `agent run` / `mission resume` behave exactly as before.
macro_rules! println {
    ($($arg:tt)*) => { crate::core::ui_events::tui_log(format!($($arg)*)) };
}
macro_rules! eprintln {
    ($($arg:tt)*) => { crate::core::ui_events::tui_warn(format!($($arg)*)) };
}

pub const CODER_SYSTEM: &str = "You are Local AI, a precise coding agent. Return ONLY machine-readable edit blocks for the MINIMAL change, nothing else.\n\nFILE EDITING RULES:\n<CREATE_FILE>FILE: path CONTENT: ... </CREATE_FILE>\n<EDIT>FILE: path SEARCH: ... REPLACE: ... </EDIT>\nRules: relative paths that already exist in the project (never invent files), SEARCH must match the file byte-for-byte, smallest change that achieves the goal, no refactoring, no new dependencies.";

/// Approval prompt with tiers: allow-once, allow-for-session, reject.
///
/// `--yes` and a prior "allow for session" skip the prompt. When the TUI
/// owns the screen the prompt becomes a modal (with `detail` diff lines);
/// a dead TUI fails closed. Non-interactive terminals fail closed with a
/// hint (re-run with `--yes`).
pub async fn confirm_or_yes(prompt: &str, yes: bool) -> Result<bool> {
    confirm_with_detail(prompt, yes, Vec::new()).await
}

pub async fn confirm_with_detail(prompt: &str, yes: bool, detail: Vec<String>) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    // Session tier: approved once for the whole run (see P1b approval tiers).
    if crate::core::config::session_approved() {
        return Ok(true);
    }
    if ui_events::has_approval_hook() {
        use ui_events::ApprovalChoice;
        match ui_events::request_approval(prompt.to_string(), detail).await {
            Some(ApprovalChoice::Once) => Ok(true),
            Some(ApprovalChoice::Session) => {
                crate::core::config::approve_session();
                Ok(true)
            }
            _ => Ok(false),
        }
    } else {
        let choices = ["Allow once", "Allow for session", "Reject"];
        match dialoguer::Select::new()
            .with_prompt(prompt)
            .items(&choices)
            .default(2)
            .interact_opt()?
        {
            Some(0) => Ok(true),
            Some(1) => {
                crate::core::config::approve_session();
                println!("  {} session approved — further prompts auto-approve", style("→").dim());
                Ok(true)
            }
            _ => {
                // Reject, Esc, or non-interactive (None).
                eprintln!("{} rejected — re-run with --yes to approve non-interactively", style("!").yellow());
                Ok(false)
            }
        }
    }
}

/// Print the unified-diff preview of `ops` before the approval prompt.
/// Read-only: rendered from current file contents, changes nothing.
pub fn print_diff_preview(proj: &crate::core::projects::Project, ops: &[core_edits::EditOp]) {
    let diff = core_edits::render_diff(proj, ops);
    if diff.is_empty() {
        return;
    }
    println!("  {} diff preview:", style("±").dim());
    for line in diff.iter().take(80) {
        let styled = if let Some(rest) = line.strip_prefix('+') {
            style(format!("    +{}", rest)).green().to_string()
        } else if let Some(rest) = line.strip_prefix('-') {
            style(format!("    -{}", rest)).red().to_string()
        } else if line.starts_with('!') {
            style(format!("    {}", line)).yellow().to_string()
        } else {
            format!("    {}", line)
        };
        println!("{}", styled);
    }
    if diff.len() > 80 {
        println!("    {} ({} more lines)", style("…").dim(), diff.len() - 80);
    }
}

/// Execution inputs for one (possibly resumed) run.
pub struct ExecCtx {
    pub goal: String,
    pub test_cmd: Option<String>,
    pub model: Option<String>,
    pub provider_kind: ProviderKind,
    pub provider_url: String,
    pub yes: bool,
    pub budget: core_agents::Budget,
    /// Stop after this many steps in *this* invocation (simulated kill / partial run).
    pub max_steps_this_run: Option<u32>,
}

/// Mutable run state — serializable into a [`crate::core::missions::Mission`].
#[derive(Debug, Default)]
pub struct RunState {
    /// Parallel to `graph.steps`: full status entries (`{"id","status",…}`).
    pub step_statuses: Vec<serde_json::Value>,
    pub applied: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub tests: Vec<serde_json::Value>,
}

/// Why the executor stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Finished,
    /// Budget bound hit (`steps`, `tools` or `wall`).
    BudgetExceeded(String),
    /// `max_steps_this_run` reached — resume later.
    StepCapReached,
}

pub struct RunOutcome {
    pub stop: StopReason,
    pub steps_this_run: u32,
    pub tool_calls: u32,
    pub tests_green: bool,
}

/// Statuses that mean "already executed, never re-run".
fn status_is_terminal(status: &str) -> bool {
    !matches!(status, "pending")
}

fn status_of(state: &RunState, step_id: &str) -> Option<String> {
    state
        .step_statuses
        .iter()
        .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(step_id))
        .and_then(|s| s.get("status").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

fn upsert_status(state: &mut RunState, entry: serde_json::Value) {
    let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let status = entry.get("status").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    if let Some(slot) = state
        .step_statuses
        .iter_mut()
        .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
    {
        *slot = entry;
    } else {
        state.step_statuses.push(entry);
    }
    ui_events::emit(ui_events::UiEvent::StepDone { id, status });
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub async fn run_graph(
    proj: &crate::core::projects::Project,
    files: &[crate::core::fs::ProjectFile],
    relevant_paths: &[String],
    graph: &core_plan::PlanGraph,
    ctx: &ExecCtx,
    cfg: &AppConfig,
    state: &mut RunState,
    orch: &mut core_agents::Orchestrator,
    trace_path: &std::path::Path,
    mission_id: &str,
    on_step: Option<std::sync::Arc<dyn Fn(&str, &str) + Send + Sync>>,
) -> Result<RunOutcome> {
    let approve_dangerous = ctx.budget.approve_dangerous;
    let mut last_failures: Vec<core_debug::Failure> = Vec::new();
    let mut steps_this_run: u32 = 0;
    let mut stop = StopReason::Finished;

    // Checkpoint (P1d): stash the dirty tree before mutating. Best-effort —
    // never blocks the run; `git rollback` restores the newest checkpoint.
    if let Some(msg) = crate::core::git::checkpoint_before_run(proj, mission_id) {
        println!("  {} checkpoint: {}", style("◈").dim(), style(msg).dim());
    }
    ui_events::emit(ui_events::UiEvent::AgentStart {
        goal: ctx.goal.clone(),
        steps: graph.steps.len(),
        max_steps: ctx.budget.max_steps,
    });
    ui_events::emit(ui_events::UiEvent::PlanCreated {
        steps: graph
            .steps
            .iter()
            .map(|s| (s.id.clone(), s.title.clone(), s.kind.to_string()))
            .collect(),
    });

    for step in &graph.steps {
        // Resume: skip steps finished by an earlier invocation (no budget consumed).
        if let Some(prev) = status_of(state, &step.id) {
            if status_is_terminal(&prev) {
                println!("  {} [{}] already {} — skipped", style("↷").dim(), step.id, prev);
                continue;
            }
        }
        if let Some(cap) = ctx.max_steps_this_run {
            if steps_this_run >= cap {
                stop = StopReason::StepCapReached;
                break;
            }
        }
        if let Err(e) = orch.check_budget() {
            stop = StopReason::BudgetExceeded(e.to_string());
            break;
        }
        orch.record_step();
        steps_this_run += 1;
        let header = format!("[{}] {} ({})", step.id, step.title, step.kind);
        println!("\n{} {}", style("▶").cyan().bold(), style(&header).bold());
        ui_events::emit(ui_events::UiEvent::StepStart {
            id: step.id.clone(),
            title: step.title.clone(),
            kind: step.kind.to_string(),
        });

        match step.kind {
            core_plan::StepKind::Inspect => {
                let total = files.len();
                let dirs = files.iter().filter(|f| f.is_directory).count();
                println!("  {} {} files ({} dirs)", style("✓ researcher").green(), total - dirs, dirs);
                ui_events::emit(ui_events::UiEvent::ToolActivity {
                    agent: "researcher".into(),
                    action: "inspect".into(),
                    detail: format!("{} files, {} dirs", total - dirs, dirs),
                    status: "done".into(),
                });
                orch.record_tool_calls(1);
                core_agents::append_trace(
                    trace_path,
                    &core_agents::TraceEntry::new(
                        mission_id,
                        "researcher",
                        "inspect",
                        &step.id,
                        &format!("{} files, {} dirs", total - dirs, dirs),
                        "done",
                    ),
                )?;
                upsert_status(state, serde_json::json!({"id": step.id, "status": "done"}));
            }
            core_plan::StepKind::Context => {
                if step.files.is_empty() {
                    println!("  {} no relevant files ranked", style("→").dim());
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "researcher", "context", &step.id, "no files", "done"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "done"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "done");
                    }
                    continue;
                }
                let mut loaded = 0usize;
                let mut missing = Vec::new();
                for f in &step.files {
                    if orch.check_budget().is_err() {
                        break;
                    }
                    match core_fs::read_project_file(proj, f) {
                        Ok(c) => {
                            loaded += 1;
                            println!("  {} {} ({} chars)", style("✓").green(), f, c.len());
                        }
                        Err(_) => missing.push(f.clone()),
                    }
                    orch.record_tool_calls(1);
                }
                for m in &missing {
                    println!("  {} {} (unreadable — skipped)", style("!").yellow(), m);
                }
                let st = if missing.is_empty() { "done" } else { "partial" };
                ui_events::emit(ui_events::UiEvent::ToolActivity {
                    agent: "researcher".into(),
                    action: "context".into(),
                    detail: format!("loaded {}/{}: {}", loaded, step.files.len(), step.files.join(", ")),
                    status: st.into(),
                });
                core_agents::append_trace(
                    trace_path,
                    &core_agents::TraceEntry::new(
                        mission_id,
                        "researcher",
                        "context",
                        &step.files.join(","),
                        &format!("loaded {}/{}", loaded, step.files.len()),
                        st,
                    ),
                )?;
                upsert_status(state, serde_json::json!({"id": step.id, "status": st}));
            }
            core_plan::StepKind::Test => {
                let cmd = ctx.test_cmd.clone().or_else(|| step.cmd.clone());
                let Some(cmd) = cmd else {
                    println!("  {} no command detected — verify manually", style("→").dim());
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "tester", "test", &step.id, "no cmd", "manual"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "manual"}));
                    state.tests.push(serde_json::json!({"step": step.id, "result": "manual"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "manual");
                    }
                    continue;
                };
                // Kill-switch first (even --yes still needs --approve dangerous).
                if let Err(e) = core_agents::check_command_allowed(&cmd, approve_dangerous) {
                    eprintln!("  {} {}", style("✗ blocked").red(), e);
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "tester", "test", &cmd, &e.to_string(), "blocked"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "blocked"}));
                    state.tests.push(serde_json::json!({"step": step.id, "cmd": cmd, "result": "blocked"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "blocked");
                    }
                    continue;
                }
                if !confirm_or_yes(&format!("Run `{}`?", cmd), ctx.yes).await? {
                    println!("  {} rejected by user", style("✗").red());
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "tester", "test", &cmd, "rejected", "rejected"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "rejected"}));
                    state.tests.push(serde_json::json!({"step": step.id, "cmd": cmd, "result": "rejected"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "rejected");
                    }
                    continue;
                }
                if orch.check_budget().is_err() {
                    steps_this_run -= 1;
                    orch.steps_used -= 1;
                    stop = StopReason::BudgetExceeded("budget exceeded before test exec".to_string());
                    break;
                }
                let result = core_fs::run_project_command(proj, &cmd)?;
                orch.record_tool_calls(1);
                let combined = format!("{}\n{}", result.stdout, result.stderr);
                if result.success {
                    println!("  {} exit {}", style("✓ PASS").green().bold(), result.exit_code.unwrap_or(0));
                    ui_events::emit(ui_events::UiEvent::TestResult {
                        step: step.id.clone(),
                        passed: true,
                        summary: format!("`{}` exit {}", cmd, result.exit_code.unwrap_or(0)),
                    });
                    last_failures.clear();
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "tester", "test", &cmd, &core_debug::tail_lines(&combined, 10), "pass"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "pass"}));
                    state.tests.push(serde_json::json!({"step": step.id, "cmd": cmd, "result": "pass"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "pass");
                    }
                } else {
                    println!("  {} exit {}", style("✗ FAIL").red().bold(), result.exit_code.unwrap_or(1));
                    last_failures = core_debug::parse_failures(&combined);
                    ui_events::emit(ui_events::UiEvent::TestResult {
                        step: step.id.clone(),
                        passed: false,
                        summary: format!(
                            "`{}` exit {} — {} failure(s){}",
                            cmd,
                            result.exit_code.unwrap_or(1),
                            last_failures.len(),
                            last_failures.first().map(|f| format!(
                                ": {}:{}",
                                f.file.as_deref().unwrap_or("?"),
                                f.line.map(|l| l.to_string()).unwrap_or_else(|| "?".into())
                            )).unwrap_or_default()
                        ),
                    });
                    if !last_failures.is_empty() {
                        println!("  {} parsed {} failure(s):", style("→").dim(), last_failures.len());
                        for f in last_failures.iter().take(5) {
                            match (&f.file, f.line) {
                                (Some(p), Some(l)) => println!("    {} {}:{} — {}", style(f.kind.as_str()).yellow(), p, l, f.message),
                                (Some(p), None) => println!("    {} {} — {}", style(f.kind.as_str()).yellow(), p, f.message),
                                _ => println!("    {} {}", style(f.kind.as_str()).yellow(), f.message),
                            }
                        }
                    }
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "tester/debugger", "test", &cmd, &core_debug::tail_lines(&combined, 30), "fail"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "fail"}));
                    state.tests.push(serde_json::json!({"step": step.id, "cmd": cmd, "result": "fail", "failures": last_failures}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "fail");
                    }
                }
                continue;
            }
            core_plan::StepKind::Edit | core_plan::StepKind::Create => {
                let Some(model_id) = ctx.model.clone() else {
                    println!("  {} no model — MANUAL: propose <EDIT> for {} yourself", style("→").dim(), step.files.join(","));
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "coder", "edit", &step.files.join(","), "no model — manual", "manual"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "manual"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "manual");
                    }
                    continue;
                };
                // Build coder prompt from goal + failures + file contents.
                let target_files = if step.files.is_empty() {
                    relevant_paths.iter().take(3).cloned().collect::<Vec<_>>()
                } else {
                    step.files.clone()
                };
                let mut ctx_parts = Vec::new();
                for f in target_files.iter().take(5) {
                    if orch.check_budget().is_err() {
                        break;
                    }
                    if let Ok(c) = core_fs::read_project_file(proj, f) {
                        let capped = if c.len() > 6000 { format!("{}…[truncated]", &c[..6000]) } else { c };
                        ctx_parts.push(format!("FILE: {}\n{}", f, capped));
                    }
                    orch.record_tool_calls(1);
                }
                let failure_text = if last_failures.is_empty() {
                    "no prior test failure".to_string()
                } else {
                    last_failures
                        .iter()
                        .take(5)
                        .map(|f| {
                            format!(
                                "- [{}] {}:{} — {}",
                                f.kind,
                                f.file.as_deref().unwrap_or("?"),
                                f.line.map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                                f.message
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let conventions = crate::core::conventions::conventions_block(proj)
                    .map(|b| format!("\n\n{}", b))
                    .unwrap_or_default();
                let user_msg = format!(
                    "Goal: {}\nStep: {} — {}\nTarget files: {}\n\nPrior test failures:\n{}\n\nProject files:\n{}{}\n\nReturn ONLY the <EDIT>/<CREATE_FILE> blocks for the minimal change.",
                    ctx.goal,
                    step.id,
                    step.title,
                    target_files.join(", "),
                    failure_text,
                    ctx_parts.join("\n\n---\n\n"),
                    conventions
                );
                println!("  {} asking {} for {}…", style("→").dim(), model_id, step.id);
                if orch.check_budget().is_err() {
                    steps_this_run -= 1;
                    orch.steps_used -= 1;
                    stop = StopReason::BudgetExceeded("budget exceeded before coder call".to_string());
                    break;
                }
                let messages = vec![
                    provider::ChatMessage { role: "system".into(), content: CODER_SYSTEM.to_string() },
                    provider::ChatMessage { role: "user".into(), content: user_msg.clone() },
                ];
                let mut sink = |chunk: &str| {
                    ui_events::emit(ui_events::UiEvent::ModelChunk { text: chunk.to_string() });
                };
                let answer = provider::stream_chat_unified(
                    &ctx.provider_kind,
                    &ctx.provider_url,
                    cfg,
                    &model_id,
                    messages,
                    0.2,
                    &mut sink,
                )
                .await
                .unwrap_or_else(|e| format!("(model error: {})", e));
                orch.record_tool_calls(1);
                core_agents::append_trace(
                    trace_path,
                    &core_agents::TraceEntry::new(mission_id, "coder", "propose", &user_msg[..user_msg.len().min(2000)], &answer, "proposed"),
                )?;

                // Verifier + security review before touching disk.
                let current_files = core_fs::list_project_files(proj).unwrap_or_else(|_| files.to_vec());
                if !current_files.is_empty() {
                    let report = verifier::verify_response_hybrid(&answer, proj, &current_files, "strict", false);
                    if !report.invented_files.is_empty() {
                        eprintln!(
                            "  {} model referenced unknown files (will be refused): {}",
                            style("⚠").yellow(),
                            report.invented_files.join(", ")
                        );
                    }
                }
                let ops = core_edits::parse_edit_blocks(&answer);
                if ops.is_empty() {
                    println!("  {} model returned no edit blocks — marking manual", style("!").yellow());
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "coder", "apply", &step.id, "no edit blocks", "manual"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "manual"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "manual");
                    }
                    continue;
                }
                println!("  {} proposed {} op(s):", style("→").dim(), ops.len());
                for op in &ops {
                    println!("    {} — {}", op.kind(), op.path());
                }
                let preview = core_edits::apply_ops(proj, &ops, true)?;
                for (p, reason) in &preview.skipped {
                    eprintln!("    {} {} — would refuse: {}", style("✗").red(), p, reason);
                }
                if preview.applied.is_empty() {
                    eprintln!("  {} all ops refused — continuing to next step", style("✗").red());
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "coder", "apply", &step.id, "all refused", "refused"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "refused"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "refused");
                    }
                    continue;
                }
                print_diff_preview(proj, &ops);
                ui_events::emit(ui_events::UiEvent::EditProposed {
                    step: step.id.clone(),
                    files: ops.iter().map(|o| o.path().to_string()).collect(),
                    diff: core_edits::render_diff(proj, &ops),
                });
                // Reviewer pre-check on the payload (secrets/injection/traversal).
                let sec = core_agents::security_review(&ops, &answer);
                if !sec.passed {
                    eprintln!(
                        "  {} reviewer pre-check FAILED: secrets={:?} traversal={:?}",
                        style("✗").red(),
                        sec.secrets_found,
                        sec.traversal_attempts
                    );
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(
                            mission_id,
                            "reviewer",
                            "precheck",
                            &step.id,
                            &serde_json::to_string(&sec).unwrap_or_default(),
                            "blocked",
                        ),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "blocked"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "blocked");
                    }
                    continue;
                }
                if !confirm_with_detail(
                    &format!("Apply {} edit(s) for {}?", preview.applied.len(), step.id),
                    ctx.yes,
                    core_edits::render_diff(proj, &ops),
                )
                .await?
                {
                    println!("  {} rejected by user", style("✗").red());
                    core_agents::append_trace(
                        trace_path,
                        &core_agents::TraceEntry::new(mission_id, "coder", "apply", &step.id, "rejected", "rejected"),
                    )?;
                    upsert_status(state, serde_json::json!({"id": step.id, "status": "rejected"}));
                    if let Some(hook) = &on_step {
                        hook(&step.id, "rejected");
                    }
                    continue;
                }
                let report = core_edits::apply_ops(proj, &ops, false)?;
                for a in &report.applied {
                    println!("    {} {}", style("✓").green(), a);
                    state.applied.push(a.clone());
                }
                for (p, reason) in &report.skipped {
                    eprintln!("    {} {} — {}", style("✗").red(), p, reason);
                    state.skipped.push((p.clone(), reason.clone()));
                }
                core_agents::append_trace(
                    trace_path,
                    &core_agents::TraceEntry::new(
                        mission_id,
                        "coder",
                        "apply",
                        &step.id,
                        &serde_json::to_string(&serde_json::json!({"applied": report.applied, "skipped": report.skipped})).unwrap_or_default(),
                        "done",
                    ),
                )?;
                ui_events::emit(ui_events::UiEvent::EditApplied {
                    step: step.id.clone(),
                    applied: report.applied.clone(),
                });
                upsert_status(state, serde_json::json!({"id": step.id, "status": "done", "applied": report.applied.len()}));
            }
            core_plan::StepKind::Review | core_plan::StepKind::Manual => {
                // Reviewer: verifier over the mission so far + security checklist.
                let current_files = core_fs::list_project_files(proj).unwrap_or_default();
                let summary_text = format!(
                    "Mission {} goal {} applied {}",
                    mission_id,
                    ctx.goal,
                    state.applied.join("; ").chars().take(1000).collect::<String>()
                );
                let mut sec = core_agents::security_review(&[], &summary_text);
                sec = core_agents::attach_verifier_invented(sec, proj, &current_files, &summary_text, "balanced");
                // Secret scan over actually-written files (paths only when clean).
                let mut dirty_notes = Vec::new();
                for a in state.applied.iter().take(10) {
                    let path = a.rsplit(' ').next().unwrap_or(a).to_string();
                    if let Ok(content) = core_fs::read_project_file(proj, &path) {
                        let r = core_agents::security_review(&[], &content);
                        if !r.secrets_found.is_empty() {
                            dirty_notes.push(format!("{}: {}", path, r.secrets_found.join("; ")));
                        }
                    }
                }
                if !dirty_notes.is_empty() {
                    sec.passed = false;
                    sec.secrets_found.extend(dirty_notes);
                }
                if sec.passed && sec.invented_files.is_empty() {
                    println!("  {} reviewer: no secrets/traversal/invented files", style("✓").green());
                } else {
                    eprintln!(
                        "  {} reviewer flagged: secrets={:?} invented={:?} traversal={:?}",
                        style("⚠").yellow(),
                        sec.secrets_found,
                        sec.invented_files,
                        sec.traversal_attempts
                    );
                }
                if !sec.injection_risks.is_empty() {
                    eprintln!("  {} injection sinks (advisory): {}", style("→").dim(), sec.injection_risks.join(", "));
                }
                let st = if sec.passed { "pass" } else { "flagged" };
                ui_events::emit(ui_events::UiEvent::Review {
                    passed: sec.passed,
                    summary: if sec.passed {
                        "no secrets/traversal/invented files".to_string()
                    } else {
                        format!(
                            "secrets={:?} invented={:?} traversal={:?}",
                            sec.secrets_found, sec.invented_files, sec.traversal_attempts
                        )
                    },
                });
                core_agents::append_trace(
                    trace_path,
                    &core_agents::TraceEntry::new(
                        mission_id,
                        "reviewer",
                        "review",
                        &step.id,
                        &serde_json::to_string(&sec).unwrap_or_default(),
                        st,
                    ),
                )?;
                upsert_status(state, serde_json::json!({"id": step.id, "status": st}));
            }
        }
        if let Some(hook) = &on_step {
            let st = status_of(state, &step.id).unwrap_or_else(|| "done".to_string());
            hook(&step.id, &st);
        }
    }

    let tests_green = !state.tests.is_empty()
        && state.tests.iter().all(|t| t.get("result").and_then(|r| r.as_str()) == Some("pass"));
    ui_events::emit(ui_events::UiEvent::AgentComplete {
        green: tests_green,
        summary: format!(
            "{} steps this run, {} tool calls, {} file(s) applied",
            steps_this_run,
            orch.tool_calls,
            state.applied.len()
        ),
    });
    Ok(RunOutcome {
        stop,
        steps_this_run,
        tool_calls: orch.tool_calls,
        tests_green,
    })
}

/// Ranked relevant file paths for the coder fallback (top 8).
pub fn relevant_paths_for(
    goal: &str,
    files: &[crate::core::fs::ProjectFile],
) -> Vec<String> {
    let intent = intelligence::detect_intent(goal);
    intelligence::rank_relevant_files(goal, files, &intent)
        .into_iter()
        .take(8)
        .map(|r| r.file.path)
        .collect()
}
