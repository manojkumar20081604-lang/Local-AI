//! Mission system — the command-center feel (Phase 4.1).
//!
//! ```text
//! MISSION #024 — Build authentication — ████████░░ 80%
//! ✓ Planner ✓ Researcher ✓ Coder ✓ Tester → Security Reviewer
//! Files: 7 modified | Tests: 31/31 | Current: security review
//! ```
//!
//! A mission is persisted Phase-2 orchestrator state: the plan graph plus
//! per-step statuses, budget, approvals and the trace path. Records live in
//! the OS data dir next to `projects.json`
//! (`~/.local/share/com.localai.app/missions.json` on Linux); the
//! per-mission `trace.jsonl` lives in the OS cache dir next to the Phase-2
//! traces (`~/.cache/local-ai/<project-id>/missions/<mission-id>/`).
//!
//! All runs stay foreground and bounded (no daemon): `mission resume`
//! executes the remaining steps through the same executor as
//! `agent run` and persists state after every step, so a killed run can be
//! resumed to completion. The frontend later renders TASKS/CHANGES/TERMINAL/
//! LOGS from these same files.
//!
//! `LOCAL_AI_MISSIONS_FILE` overrides the registry path (used by tests).

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::agents::Budget;
use super::plan::PlanGraph;
use super::projects::Project;

/// Terminal states: nothing left to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissionStatus {
    Pending,
    Running,
    Done,
    Cancelled,
    Failed,
}

impl std::fmt::Display for MissionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        };
        write!(f, "{}", s)
    }
}

impl MissionStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Failed)
    }
}

/// One skipped edit (path + refusal reason).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub path: String,
    pub reason: String,
}

/// One persisted mission: plan graph + execution state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    /// Human number (`#024`), unique per registry.
    pub number: u32,
    /// Stable id (`024-a1b2c3d4`), also the cache trace-dir name.
    pub id: String,
    pub goal: String,
    pub intent: String,
    pub project_id: String,
    pub project_name: String,
    /// Canonical folder at creation (needed to resume after restart).
    #[serde(default)]
    pub folder_path: Option<String>,
    pub status: MissionStatus,
    pub plan: PlanGraph,
    /// Parallel to `plan.steps`: `pending|done|partial|pass|fail|manual|refused|blocked|rejected|flagged`.
    pub step_statuses: Vec<String>,
    pub applied: Vec<String>,
    #[serde(default)]
    pub skipped: Vec<SkippedEntry>,
    pub tests: Vec<serde_json::Value>,
    pub tests_green: bool,
    pub budget: Budget,
    pub approve_dangerous: bool,
    pub test_cmd: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub trace_path: PathBuf,
}

impl Mission {
    /// 0.0–1.0 fraction of steps past `pending`.
    pub fn progress(&self) -> f32 {
        if self.plan.steps.is_empty() {
            return 1.0;
        }
        let done = self
            .step_statuses
            .iter()
            .filter(|s| *s != "pending")
            .count();
        done as f32 / self.plan.steps.len() as f32
    }

    /// Id of the next `pending` step, if any.
    pub fn current_step(&self) -> Option<String> {
        self.plan
            .steps
            .iter()
            .zip(self.step_statuses.iter())
            .find(|(_, s)| *s == "pending")
            .map(|(step, _)| step.id.clone())
    }
}

fn registry_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("LOCAL_AI_MISSIONS_FILE") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .context("Could not determine data directory")?;
    let dir = base.join("com.localai.app");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("missions.json"))
}

pub fn read_missions() -> Result<Vec<Mission>> {
    let path = registry_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path).context("Failed to read missions.json")?;
    if contents.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&contents).context("Failed to parse missions.json")
}

pub fn write_missions(missions: &[Mission]) -> Result<()> {
    let path = registry_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(missions)?)?;
    Ok(())
}

/// Create + persist a mission from a plan graph. Returns the record.
/// The trace dir is created with an empty `trace.jsonl`.
#[allow(clippy::too_many_arguments)]
pub fn create_mission_record(
    project: &Project,
    goal: &str,
    intent: &str,
    plan: PlanGraph,
    budget: Budget,
    approve_dangerous: bool,
    test_cmd: Option<String>,
    model: Option<String>,
) -> Result<Mission> {
    let mut missions = read_missions()?;
    let number = missions.iter().map(|m| m.number).max().unwrap_or(0) + 1;
    let id = format!("{:03}-{}", number, &uuid::Uuid::new_v4().to_string()[..8]);
    let base = dirs::cache_dir().context("No cache dir")?;
    let dir = base.join("local-ai").join(&project.id).join("missions").join(&id);
    fs::create_dir_all(&dir)?;
    let trace_path = dir.join("trace.jsonl");
    if !trace_path.exists() {
        fs::write(&trace_path, "")?;
    }
    let now = Utc::now().to_rfc3339();
    let n_steps = plan.steps.len();
    let mission = Mission {
        number,
        id,
        goal: goal.to_string(),
        intent: intent.to_string(),
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        folder_path: project.folder_path.clone(),
        status: MissionStatus::Pending,
        step_statuses: vec!["pending".to_string(); n_steps],
        applied: Vec::new(),
        skipped: Vec::new(),
        tests: Vec::new(),
        tests_green: false,
        plan,
        budget,
        approve_dangerous,
        test_cmd,
        model,
        created_at: now.clone(),
        updated_at: now,
        trace_path,
    };
    missions.push(mission.clone());
    write_missions(&missions)?;
    Ok(mission)
}

pub fn get_mission(id_or_number: &str) -> Result<Mission> {
    let missions = read_missions()?;
    // Accept `#024`, `024`, `24`, or the full id.
    let trimmed = id_or_number.trim().trim_start_matches('#');
    if let Ok(n) = trimmed.parse::<u32>() {
        if let Some(m) = missions.iter().find(|m| m.number == n) {
            return Ok(m.clone());
        }
    }
    missions
        .into_iter()
        .find(|m| m.id == trimmed || m.id.starts_with(trimmed))
        .with_context(|| format!("Mission not found: {}", id_or_number))
}

/// Upsert by id. Refreshes `updated_at`.
pub fn save_mission(mission: &Mission) -> Result<()> {
    let mut missions = read_missions()?;
    let mut updated = mission.clone();
    updated.updated_at = Utc::now().to_rfc3339();
    if let Some(slot) = missions.iter_mut().find(|m| m.id == updated.id) {
        *slot = updated;
    } else {
        missions.push(updated);
    }
    write_missions(&missions)
}

/// Mark a step finished and persist. `status` is the executor status word.
pub fn record_step_status(mission: &mut Mission, step_id: &str, status: &str) -> Result<()> {
    if let Some(pos) = mission.plan.steps.iter().position(|s| s.id == step_id) {
        if pos < mission.step_statuses.len() {
            mission.step_statuses[pos] = status.to_string();
        }
    }
    save_mission(mission)
}

/// Progress bar for `mission show`: `████████░░ 80%`.
pub fn progress_bar(progress: f32) -> String {
    let filled = (progress * 10.0).round().clamp(0.0, 10.0) as usize;
    format!(
        "{}{} {:>3}%",
        "█".repeat(filled),
        "░".repeat(10 - filled),
        (progress * 100.0).round() as u32
    )
}
