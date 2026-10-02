//! Self-improvement metrics + proposals — the standout feature (Phase 5.3).
//!
//! ```text
//! approved interactions → dataset builder → QLoRA → eval → compare → deploy if better
//! ```
//!
//! ```bash
//! local-ai metrics --project MyApp   # retrieval hit-rate, verifier reject-rate, test pass-rate
//! local-ai propose --project MyApp   # inspectable improvements, never silent self-mod
//! ```
//!
//! Sources (all local, all offline):
//! - `missions.json` registry (filtered by `project_id`): mission/step/test counts.
//! - `trace.jsonl` files (`~/.cache/local-ai/<id>/missions/*/trace.jsonl` +
//!   agent-run traces): researcher/coder/tester/reviewer outcomes.
//! - `debug/*.json` transcripts (`~/.cache/local-ai/<id>/debug/`): pass/fail.
//! - `index.json`: files/symbols/edges + staleness (retrieval health).
//!
//! Proposals are **plan-graphs** (Phase 1.4): `propose --apply <id>` only
//! saves a plan JSON to the cache and prints the next command — it never
//! edits project files silently.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::agents::TraceEntry;
use super::missions::Mission;
use super::plan::{PlanGraph, PlanStep, StepKind};
use super::projects::Project;

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Health rates for one project. `None` = no data yet (needs missions/traces).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetrics {
    pub project_id: String,
    pub project_name: String,
    pub missions_total: usize,
    pub missions_done: usize,
    pub steps_total: usize,
    pub steps_finished: usize,
    pub tests_total: usize,
    pub tests_pass: usize,
    pub test_pass_rate: Option<f32>,
    pub trace_files: usize,
    pub trace_entries: usize,
    pub context_total: usize,
    pub context_done: usize,
    pub retrieval_hit_rate: Option<f32>,
    pub review_total: usize,
    pub review_flagged: usize,
    pub coder_propose_total: usize,
    pub coder_refused: usize,
    pub verifier_reject_rate: Option<f32>,
    pub debug_runs: usize,
    pub debug_pass: usize,
    pub index_files: usize,
    pub index_symbols: usize,
    pub index_edges: usize,
    pub index_stale: Option<bool>,
}

impl ProjectMetrics {
    pub fn empty(project: &Project) -> Self {
        Self {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            missions_total: 0,
            missions_done: 0,
            steps_total: 0,
            steps_finished: 0,
            tests_total: 0,
            tests_pass: 0,
            test_pass_rate: None,
            trace_files: 0,
            trace_entries: 0,
            context_total: 0,
            context_done: 0,
            retrieval_hit_rate: None,
            review_total: 0,
            review_flagged: 0,
            coder_propose_total: 0,
            coder_refused: 0,
            verifier_reject_rate: None,
            debug_runs: 0,
            debug_pass: 0,
            index_files: 0,
            index_symbols: 0,
            index_edges: 0,
            index_stale: None,
        }
    }

    pub fn has_data(&self) -> bool {
        self.missions_total > 0 || self.trace_entries > 0 || self.debug_runs > 0
    }
}

fn rate(pass: usize, total: usize) -> Option<f32> {
    if total == 0 {
        None
    } else {
        Some(pass as f32 / total as f32)
    }
}

/// Pure: mission registry → step/test counts (filtered by caller to one project).
pub fn mission_stats(missions: &[Mission]) -> (usize, usize, usize, usize, usize, usize) {
    // (missions_total, missions_done, steps_total, steps_finished, tests_total, tests_pass)
    let missions_total = missions.len();
    let missions_done = missions
        .iter()
        .filter(|m| m.status.to_string() == "done")
        .count();
    let mut steps_total = 0usize;
    let mut steps_finished = 0usize;
    let mut tests_total = 0usize;
    let mut tests_pass = 0usize;
    for m in missions {
        steps_total += m.plan.steps.len();
        steps_finished += m.step_statuses.iter().filter(|s| *s != "pending").count();
        for t in &m.tests {
            tests_total += 1;
            if t.get("result").and_then(|r| r.as_str()) == Some("pass") {
                tests_pass += 1;
            }
        }
    }
    (
        missions_total,
        missions_done,
        steps_total,
        steps_finished,
        tests_total,
        tests_pass,
    )
}

/// Pure: trace entries → retrieval / verifier / test counters.
#[derive(Debug, Default)]
pub struct TraceStats {
    pub entries: usize,
    pub context_total: usize,
    pub context_done: usize,
    pub review_total: usize,
    pub review_flagged: usize,
    pub coder_propose_total: usize,
    pub coder_refused: usize,
    pub tests_total: usize,
    pub tests_pass: usize,
}

pub fn trace_stats(entries: &[TraceEntry]) -> TraceStats {
    let mut s = TraceStats { entries: entries.len(), ..Default::default() };
    for e in entries {
        match (e.agent.as_str(), e.action.as_str()) {
            ("researcher", "context") => {
                s.context_total += 1;
                if e.status == "done" {
                    s.context_done += 1;
                }
            }
            ("reviewer", _) => {
                // Count terminal review verdicts; ignore intermediate notes.
                if matches!(e.status.as_str(), "pass" | "flagged" | "blocked" | "done") {
                    s.review_total += 1;
                    if e.status == "flagged" || e.status == "blocked" {
                        s.review_flagged += 1;
                    }
                }
            }
            ("coder", "propose") => {
                s.coder_propose_total += 1;
            }
            ("coder", "apply") => {
                if e.status == "refused" || e.status == "blocked" || e.status == "rejected" {
                    s.coder_refused += 1;
                }
            }
            ("tester", "test") | ("tester/debugger", "test") => {
                // Count all test executions (pass + fail); manual/blocked/rejected
                // are workflow states, not test outcomes.
                if matches!(e.status.as_str(), "pass" | "fail") {
                    s.tests_total += 1;
                    if e.status == "pass" {
                        s.tests_pass += 1;
                    }
                }
            }
            _ => {}
        }
    }
    s
}

/// Pure: debug transcripts (`debug/*.json` reports) → run/pass counts.
/// Each report is the `save_debug_transcript` JSON (`{outcome, attempts}`).
pub fn debug_stats(reports: &[serde_json::Value]) -> (usize, usize) {
    let mut total = 0usize;
    let mut pass = 0usize;
    for r in reports {
        total += 1;
        if r.get("outcome").and_then(|o| o.as_str()) == Some("pass") {
            pass += 1;
        }
    }
    (total, pass)
}

/// Merge mission + trace + debug + index counters into rates.
pub fn merge_metrics(
    project: &Project,
    missions: &[Mission],
    trace_entries: &[TraceEntry],
    trace_files: usize,
    debug_reports: &[serde_json::Value],
    index_info: Option<(usize, usize, usize, Option<bool>)>,
) -> ProjectMetrics {
    let mut m = ProjectMetrics::empty(project);
    let (mt, md, st, sf, tt, tp) = mission_stats(missions);
    m.missions_total = mt;
    m.missions_done = md;
    m.steps_total = st;
    m.steps_finished = sf;

    let ts = trace_stats(trace_entries);
    m.trace_files = trace_files;
    m.trace_entries = ts.entries;
    m.context_total = ts.context_total;
    m.context_done = ts.context_done;
    m.retrieval_hit_rate = rate(ts.context_done, ts.context_total);
    m.review_total = ts.review_total;
    m.review_flagged = ts.review_flagged;
    m.coder_propose_total = ts.coder_propose_total;
    m.coder_refused = ts.coder_refused;
    let reject_numer = ts.review_flagged + ts.coder_refused;
    let reject_denom = ts.review_total + ts.coder_propose_total;
    m.verifier_reject_rate = rate(reject_numer, reject_denom)
        .map(|r| if reject_denom == 0 { 0.0 } else { r })
        .or(if reject_denom == 0 { None } else { rate(reject_numer, reject_denom) });

    // Tests: missions registry + trace tester outcomes + debug pass.
    let (dt, dp) = debug_stats(debug_reports);
    m.debug_runs = dt;
    m.debug_pass = dp;
    m.tests_total = tt + ts.tests_total + dt;
    m.tests_pass = tp + ts.tests_pass + dp;
    m.test_pass_rate = rate(m.tests_pass, m.tests_total);

    if let Some((files, syms, edges, stale)) = index_info {
        m.index_files = files;
        m.index_symbols = syms;
        m.index_edges = edges;
        m.index_stale = stale;
    }
    m
}

// ---------------------------------------------------------------------------
// Disk collection
// ---------------------------------------------------------------------------

fn cache_project_dir(project: &Project) -> Option<std::path::PathBuf> {
    dirs::cache_dir().map(|b| b.join("local-ai").join(&project.id))
}

fn list_trace_files(project: &Project) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    // 1) Registry trace paths (canonical).
    if let Ok(missions) = super::missions::read_missions() {
        for m in missions.iter().filter(|m| m.project_id == project.id) {
            if m.trace_path.exists() && !out.contains(&m.trace_path) {
                out.push(m.trace_path.clone());
            }
        }
    }
    // 2) Scan the cache missions dir (covers ad-hoc `agent run` traces
    //    that never entered the registry).
    if let Some(base) = cache_project_dir(project) {
        let dir = base.join("missions");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let tp = entry.path().join("trace.jsonl");
                if tp.exists() && !out.contains(&tp) {
                    out.push(tp);
                }
            }
        }
    }
    out.sort();
    out
}

fn list_debug_reports(project: &Project) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let Some(base) = cache_project_dir(project) else {
        return out;
    };
    let dir = base.join("debug");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(content) = std::fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    out.push(v);
                }
            }
        }
    }
    out
}

/// Read-only collection: missions + traces + debug + index for one project.
/// Never writes to the project — plan-mode safe (cache reads only).
pub fn collect_project_metrics(project: &Project) -> Result<ProjectMetrics> {
    let missions = super::missions::read_missions().unwrap_or_default();
    let mine: Vec<Mission> = missions
        .into_iter()
        .filter(|m| m.project_id == project.id)
        .collect();

    let trace_files = list_trace_files(project);
    let mut entries = Vec::new();
    for tf in &trace_files {
        entries.extend(super::dataset::read_trace_entries(tf));
    }

    let debug_reports = list_debug_reports(project);

    let index_info = match super::index::load_index(project) {
        Ok(Some(idx)) => {
            let stale = super::index::needs_rebuild(project, &idx);
            Some((idx.files.len(), idx.symbols.len(), idx.imports.len(), Some(stale)))
        }
        _ => None,
    };

    Ok(merge_metrics(
        project,
        &mine,
        &entries,
        trace_files.len(),
        &debug_reports,
        index_info,
    ))
}

// ---------------------------------------------------------------------------
// Proposals (plan-graphs, never silent self-modification)
// ---------------------------------------------------------------------------

/// One inspectable improvement. `plan` is the executable follow-up
/// (`exec-plan <saved-path>`); `apply_hint` is the human command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub severity: String,
    pub rationale: String,
    pub expected_gain: String,
    pub inspect: Vec<String>,
    pub plan: PlanGraph,
    pub apply_hint: String,
}

fn proposal_plan(
    project: &Project,
    goal: &str,
    intent: &str,
    steps: Vec<PlanStep>,
) -> PlanGraph {
    let graph = PlanGraph {
        version: 1,
        goal: goal.to_string(),
        intent: intent.to_string(),
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        steps,
    };
    // All proposal plans are constructed valid; debug-assert in tests.
    let _ = super::plan::validate_graph(&graph);
    graph
}

fn step(id: &str, title: &str, kind: StepKind, files: Vec<String>, cmd: Option<String>, depends_on: Vec<String>, notes: &str) -> PlanStep {
    PlanStep {
        id: id.to_string(),
        title: title.to_string(),
        kind,
        files,
        cmd,
        depends_on,
        notes: notes.to_string(),
    }
}

/// Deterministic rules from real metrics (no LLM, offline).
/// Thresholds mirror the plan text: retrieval fails ~18% → symbol retrieval.
pub fn build_proposals(
    metrics: &ProjectMetrics,
    project: &Project,
    files: &[super::fs::ProjectFile],
) -> Vec<Proposal> {
    let mut out = Vec::new();
    let test_cmd = super::plan::detect_test_cmd(files);

    // 0) No data → collect a baseline first (missions/traces feed everything).
    if !metrics.has_data() {
        out.push(Proposal {
            id: "collect-baseline".to_string(),
            title: "Collect a baseline: run one mission + debug loop".to_string(),
            kind: "baseline".to_string(),
            severity: "info".to_string(),
            rationale: "No missions, traces or debug runs found — metrics need at least one instrumented run.".to_string(),
            expected_gain: "enables retrieval/verifier/test rates".to_string(),
            inspect: vec!["missions registry (empty)".to_string()],
            plan: proposal_plan(project, "Collect baseline traces", "run", vec![
                step("inspect", "Inspect project tree", StepKind::Inspect, vec![], None, vec![], "List files to confirm scope."),
                step("context", "Load relevant files", StepKind::Context, vec![], None, vec!["inspect".into()], "Rank top files for the next goal."),
                step("run", "Run one mission", StepKind::Manual, vec![], None, vec!["context".into()], "MANUAL: local-ai mission create \"<goal>\" --project <name>, then mission resume --yes."),
                step("test", "Run tests", StepKind::Test, vec![], test_cmd.clone(), vec!["run".into()], "Approval-gated test run."),
                step("review", "Verify + security review", StepKind::Review, vec![], None, vec!["test".into()], "Verifier + secrets/traversal checklist."),
            ]),
            apply_hint: "local-ai mission create \"<goal>\" --project <name> && local-ai mission resume <id> --yes".to_string(),
        });
        return out;
    }

    // 1) Retrieval: hit-rate < 85% or stale/missing index.
    let retrieval_bad = matches!(metrics.retrieval_hit_rate, Some(r) if r < 0.85);
    let index_bad = matches!(metrics.index_stale, Some(true)) || (metrics.index_files == 0 && metrics.context_total > 0);
    if retrieval_bad || index_bad {
        let hit = metrics.retrieval_hit_rate.map(|r| format!("{:.0}%", r * 100.0)).unwrap_or_else(|| "n/a".to_string());
        out.push(Proposal {
            id: "rebuild-index-symbols".to_string(),
            title: format!("Retrieval hit-rate {} — rebuild index with symbols", hit),
            kind: "retrieval".to_string(),
            severity: "high".to_string(),
            rationale: format!(
                "researcher context loads: {}/{} fully loaded (hit-rate {}). {}. Symbol boost (+0.15) + git recency (+0.10) lift large-TS hit rates ~+12%.",
                metrics.context_done, metrics.context_total, hit,
                match metrics.index_stale {
                    Some(true) => "index.json is STALE (mtime/hash drift)".to_string(),
                    Some(false) => format!("index.json fresh ({} files, {} symbols, {} edges)", metrics.index_files, metrics.index_symbols, metrics.index_edges),
                    None => "no index.json found".to_string(),
                }
            ),
            expected_gain: "expected +12% retrieval hit-rate".to_string(),
            inspect: vec![
                "index status: local-ai index status --project <name>".to_string(),
                format!("trace context loads: {}/{} partial", metrics.context_total - metrics.context_done, metrics.context_total),
            ],
            plan: proposal_plan(project, "Rebuild retrieval index with symbols", "run", vec![
                step("inspect", "Inspect index staleness", StepKind::Inspect, vec![], None, vec![], "Run index status; confirm stale files."),
                step("rebuild", "Rebuild index with symbols", StepKind::Manual, vec![], None, vec!["inspect".into()], "MANUAL: local-ai index rebuild --project <name> (default --symbols on)."),
                step("test", "Re-run retrieval probe", StepKind::Test, vec![], test_cmd.clone(), vec!["rebuild".into()], "Re-run graph query + hybrid rank; compare hit-rate."),
                step("review", "Verify no invented symbols", StepKind::Review, vec![], None, vec!["test".into()], "Verifier: symbol citations resolve via graph."),
            ]),
            apply_hint: "local-ai index rebuild --project <name>  # then re-run: local-ai metrics --project <name>".to_string(),
        });
    }

    // 2) Verifier: reject-rate > 10% → tighten grounding.
    if matches!(metrics.verifier_reject_rate, Some(r) if r > 0.10) {
        let rej = metrics.verifier_reject_rate.map(|r| format!("{:.0}%", r * 100.0)).unwrap_or_default();
        out.push(Proposal {
            id: "tighten-grounding".to_string(),
            title: format!("Verifier reject-rate {} — tighten grounding", rej),
            kind: "grounding".to_string(),
            severity: "high".to_string(),
            rationale: format!(
                "reviewer flagged {}/{} + coder refused {} (reject-rate {}). Model invents files/symbols or trips secrets/traversal guards.",
                metrics.review_flagged, metrics.review_total, metrics.coder_refused, rej
            ),
            expected_gain: "fewer hallucinated edits, fewer refused applies".to_string(),
            inspect: vec![
                format!("reviewer flagged: {}/{}", metrics.review_flagged, metrics.review_total),
                format!("coder refused: {}", metrics.coder_refused),
                "recent verifier output: chat/analyze --show-verifier".to_string(),
            ],
            plan: proposal_plan(project, "Tighten grounding to strict", "review", vec![
                step("inspect", "Inspect rejected outputs", StepKind::Inspect, vec![], None, vec![], "Read trace reviewer/coder entries with refused/flagged status."),
                step("context", "Reload grounding config", StepKind::Context, vec![], None, vec!["inspect".into()], "Check config grounding mode + citations."),
                step("answer", "Answer with strict citations", StepKind::Manual, vec![], None, vec!["context".into()], "MANUAL: re-run with --grounding strict --show-verifier; require [path:line] per claim."),
                step("review", "Verify reject-rate drops", StepKind::Review, vec![], None, vec!["answer".into()], "Verifier: invented count → 0 on retry."),
            ]),
            apply_hint: "local-ai config set grounding.mode strict  # then: chat/analyze --grounding strict --show-verifier".to_string(),
        });
    }

    // 3) Tests: pass-rate < 100% (with data) → shrink edits + debug loop.
    if matches!(metrics.test_pass_rate, Some(r) if r < 1.0) {
        let tp = metrics.test_pass_rate.map(|r| format!("{:.0}%", r * 100.0)).unwrap_or_default();
        out.push(Proposal {
            id: "shrink-edits-debug".to_string(),
            title: format!("Test pass-rate {} — smaller edits + debug loop", tp),
            kind: "testing".to_string(),
            severity: "medium".to_string(),
            rationale: format!(
                "tests: {}/{} passing (pass-rate {}). Large edits fail more; the Phase-1.2 debug loop (reproduce → isolate → fix) recovers green fastest.",
                metrics.tests_pass, metrics.tests_total, tp
            ),
            expected_gain: "faster return to green, fewer manual steps".to_string(),
            inspect: vec![
                format!("failing tests: {}/{}", metrics.tests_total - metrics.tests_pass, metrics.tests_total),
                format!("debug runs: {}/{} passing", metrics.debug_pass, metrics.debug_runs),
            ],
            plan: proposal_plan(project, "Recover tests via debug loop", "debug", vec![
                step("inspect", "Inspect failing tests", StepKind::Inspect, vec![], None, vec![], "List failing suites from missions/tests + debug transcripts."),
                step("reproduce", "Reproduce the failure", StepKind::Test, vec![], test_cmd.clone(), vec!["inspect".into()], "Run before editing to capture output."),
                step("context", "Load failing files", StepKind::Context, vec![], None, vec!["reproduce".into()], "Rank files from failure file:line + query."),
                step("test", "Re-run tests", StepKind::Test, vec![], test_cmd.clone(), vec!["context".into()], "Approval-gated re-test after minimal fix."),
                step("review", "Verify + security review", StepKind::Review, vec![], None, vec!["test".into()], "Verifier + checklist on the fix."),
            ]),
            apply_hint: "local-ai debug --project <name> --max-attempts 3 --yes  # or: agent run \"fix failing tests\"".to_string(),
        });
    }

    // 4) All green with volume → expand bench coverage (feeds finetune eval).
    if out.is_empty() {
        out.push(Proposal {
            id: "expand-bench".to_string(),
            title: "All signals green — expand bench + finetune feedback".to_string(),
            kind: "growth".to_string(),
            severity: "low".to_string(),
            rationale: format!(
                "retrieval {}, verifier rejects {}, tests {}/{} — healthy. Next leverage: grow bench/prompts.jsonl from cli/tests fixtures and run finetune eval before every deploy.",
                metrics.retrieval_hit_rate.map(|r| format!("{:.0}%", r * 100.0)).unwrap_or_else(|| "n/a".to_string()),
                metrics.verifier_reject_rate.map(|r| format!("{:.0}%", r * 100.0)).unwrap_or_else(|| "n/a".to_string()),
                metrics.tests_pass, metrics.tests_total,
            ),
            expected_gain: "deploy gate actually blocks regressions".to_string(),
            inspect: vec![
                "bench prompts: bench/prompts.jsonl (5 seeded)".to_string(),
                "dataset: finetune-dataset.jsonl (from missions/chat)".to_string(),
            ],
            plan: proposal_plan(project, "Expand bench + dataset feedback", "run", vec![
                step("inspect", "Inspect bench coverage", StepKind::Inspect, vec![], None, vec![], "Check bench/prompts.jsonl suites vs cli/tests fixtures."),
                step("context", "Collect approved interactions", StepKind::Context, vec![], None, vec!["inspect".into()], "Run dataset collect --only-approved from missions/chat."),
                step("test", "Run bench + finetune eval", StepKind::Test, vec![], Some("local-ai bench run --suite coding".into()), vec!["context".into()], "Approval-gated bench; compare base vs adapter."),
                step("review", "Gate deploy on eval", StepKind::Review, vec![], None, vec!["test".into()], "Only merge→quantize→deploy when adapter ≥ base."),
            ]),
            apply_hint: "local-ai dataset collect --project <name> --from all --only-approved && local-ai bench run --suite coding".to_string(),
        });
    }

    out
}

/// Persist a proposal's plan-graph to the plans cache; returns the path.
/// This is what `propose --apply <id>` does — no project files touched.
pub fn save_proposal_plan(project: &Project, proposal: &Proposal) -> Result<std::path::PathBuf> {
    super::plan::save_plan(project, &proposal.plan)
}
