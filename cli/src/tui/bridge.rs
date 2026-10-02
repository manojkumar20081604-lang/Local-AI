//! Headless agent runs for the TUI — same engine, no terminal printing.
//!
//! [`run_goal`] mirrors `agent run` (plan → `run_graph` → report) and
//! [`answer_once`] mirrors `analyze` (context → streamed answer); both emit
//! [`crate::core::ui_events::UiEvent`]s instead of printing. The engine
//! code itself is untouched — output rerouting lives in `runner.rs` behind
//! the takeover flag.

use anyhow::Result;

use crate::commands::runner;
use crate::core::{
    agents as core_agents, config::ProviderKind, fs as core_fs, intelligence,
    plan as core_plan, projects::Project, provider, ui_events,
};

/// Budget shown in the header (same defaults as `agent run`).
pub const DEFAULT_MAX_STEPS: u32 = 20;

/// Run the full plan loop for `goal`. Approvals arrive via the TUI modal
/// (`yes: false`); the TUI may abort the returned task on Ctrl+C.
pub async fn run_goal(
    proj: &Project,
    goal: &str,
    provider_kind: &ProviderKind,
    provider_url: &str,
    model_override: Option<String>,
) -> Result<String> {
    let cfg = crate::core::config::load_config().unwrap_or_default();
    let files = core_fs::list_project_files(proj)?;
    let intent = intelligence::detect_intent(goal);
    let relevant: Vec<_> = intelligence::rank_relevant_files(goal, &files, &intent)
        .into_iter()
        .take(8)
        .collect();
    let relevant_paths: Vec<String> = relevant.iter().map(|r| r.file.path.clone()).collect();
    let graph = core_plan::build_plan(goal, proj, &files, &intent.intent, &relevant);
    core_plan::validate_graph(&graph)?;

    let (mission_id, trace_path) = core_agents::create_mission(proj)?;
    let budget = core_agents::Budget {
        max_steps: DEFAULT_MAX_STEPS,
        max_tool_calls: 50,
        max_wall_secs: 600,
        approve_dangerous: false,
    };
    core_agents::append_trace(
        &trace_path,
        &core_agents::TraceEntry::new(
            &mission_id,
            "planner",
            "build_plan",
            goal,
            &serde_json::to_string(&graph).unwrap_or_default(),
            "done",
        ),
    )?;

    let model: Option<String> =
        provider::resolve_model_id(model_override, provider_kind, provider_url, &cfg).await;

    ui_events::emit(ui_events::UiEvent::ModelSwitch {
        model: model.clone().unwrap_or_else(|| "(none)".into()),
        provider: provider_kind.to_string(),
    });

    let mut orch = core_agents::Orchestrator::new(
        mission_id.clone(),
        budget.clone(),
        core_agents::ApprovalGate::Always,
    );
    let ctx = runner::ExecCtx {
        goal: goal.to_string(),
        test_cmd: None,
        model,
        provider_kind: provider_kind.clone(),
        provider_url: provider_url.to_string(),
        yes: false,
        budget,
        max_steps_this_run: None,
    };
    let mut state = runner::RunState::default();
    let outcome = runner::run_graph(
        proj, &files, &relevant_paths, &graph, &ctx, &cfg, &mut state, &mut orch,
        &trace_path, &mission_id, None,
    )
    .await?;

    let summary = format!(
        "{} steps, {} tool calls, {} file(s) applied, tests {}",
        outcome.steps_this_run,
        outcome.tool_calls,
        state.applied.len(),
        if outcome.tests_green { "green" } else { "not green" },
    );
    core_agents::append_trace(
        &trace_path,
        &core_agents::TraceEntry::new(&mission_id, "orchestrator", "finish", goal, &summary, "done"),
    )?;
    Ok(summary)
}

/// Answer a question directly (no plan loop): grounded context + streaming.
/// Chunks stream as `ModelChunk`; the full text lands as `ModelDone`.
pub async fn answer_once(
    proj: &Project,
    query: &str,
    provider_kind: &ProviderKind,
    provider_url: &str,
    model_override: Option<String>,
) -> Result<String> {
    ui_events::emit(ui_events::UiEvent::AnswerStart { query: query.to_string() });
    let cfg = crate::core::config::load_config().unwrap_or_default();
    let files = core_fs::list_project_files(proj).unwrap_or_default();
    let index_opt = crate::core::index::load_index(proj).ok().flatten();
    let graph = intelligence::load_code_graph(proj, &files, index_opt.as_ref(), |p| {
        core_fs::read_project_file(proj, p).ok()
    });
    let recency = intelligence::load_recency(proj);
    let mut context = intelligence::build_project_context_rag2(
        query, &files, index_opt.as_ref(), None, Some(&graph), Some(&recency), |p| {
            core_fs::read_project_file(proj, p).ok()
        },
    );
    if let Some(block) = crate::core::conventions::conventions_block(proj) {
        context = format!("{}\n\n{}", block, context);
    }
    let user_mem =
        crate::core::memory::load_memory(crate::core::memory::MemoryScope::User, None, None)
            .unwrap_or_default();
    let proj_mem = crate::core::memory::load_memory(
        crate::core::memory::MemoryScope::Project, Some(proj), None,
    )
    .unwrap_or_default();
    let merged = crate::core::memory::merged_context(&user_mem, &proj_mem, None);
    if !merged.is_empty() {
        context = format!("MEMORY:\n{}\n\n{}", merged, context);
    }

    let model: Option<String> =
        provider::resolve_model_id(model_override, provider_kind, provider_url, &cfg).await;
    let Some(model_id) = model else {
        anyhow::bail!("No model reachable — start Ollama or LM Studio first")
    };
    ui_events::emit(ui_events::UiEvent::ModelSwitch {
        model: model_id.clone(),
        provider: provider_kind.to_string(),
    });

    let system = format!(
        "You are Local AI, a grounded coding assistant. Use ONLY the project context. Be concise.\n\n{}",
        context
    );
    let messages = vec![
        provider::ChatMessage { role: "system".into(), content: system },
        provider::ChatMessage { role: "user".into(), content: query.to_string() },
    ];
    let mut full = String::new();
    let mut sink = |chunk: &str| {
        full.push_str(chunk);
        ui_events::emit(ui_events::UiEvent::ModelChunk { text: chunk.to_string() });
    };
    let first_err = match provider::stream_chat_unified(provider_kind, provider_url, &cfg, &model_id, messages.clone(), 0.4, &mut sink).await {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    };
    if let Some(err) = first_err {
        if provider::is_model_not_found(&err) {
            // Self-heal once: a stale/typo'd id (e.g. unpulled cloud model)
            // falls back to a servable one instead of a dead chat.
            let available = provider::list_models_unified(provider_kind, provider_url, &cfg)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|m| m.id)
                .collect::<Vec<_>>();
            if let Some(fb) = provider::fallback_model(&model_id, &available) {
                ui_events::emit(ui_events::UiEvent::Warn {
                    line: format!(
                        "Model '{}' isn't servable here — falling back to '{}' (pin it: /model {}).",
                        model_id, fb, fb
                    ),
                });
                ui_events::emit(ui_events::UiEvent::ModelSwitch {
                    model: fb.clone(),
                    provider: provider_kind.to_string(),
                });
                provider::stream_chat_unified(provider_kind, provider_url, &cfg, &fb, messages, 0.4, &mut sink)
                    .await
                    .map_err(|e2| {
                        anyhow::anyhow!(provider::friendly_error(
                            &e2.to_string(),
                            &provider_kind.to_string(),
                            &fb
                        ))
                    })?;
            } else {
                anyhow::bail!(provider::friendly_error(&err, &provider_kind.to_string(), &model_id));
            }
        } else {
            anyhow::bail!(provider::friendly_error(&err, &provider_kind.to_string(), &model_id));
        }
    }
    ui_events::emit(ui_events::UiEvent::ModelDone { full: full.clone() });
    Ok(full)
}
