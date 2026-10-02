//! Plan Mode 2.0 — deterministic execution graphs (Phase 1.4).
//!
//! A plan is a machine-readable DAG of steps built WITHOUT an LLM from
//! intent detection + relevant-file ranking. The LLM/agent fills in the
//! design details later (Phase 2); the graph structure is stable and
//! executable by `exec-plan`.
//!
//! ```json
//! { "goal": "Add auth", "steps": [
//!   { "id": "inspect", "kind": "inspect", "files": [], "cmd": null, "depends_on": [] },
//!   { "id": "test", "kind": "test", "files": [], "cmd": "cargo test", "depends_on": ["fix"] }
//! ]}
//! ```
//!
//! Plans are persisted under the OS cache dir next to the retrieval index:
//! `~/.cache/local-ai/<project-id>/plans/<slug>-<ts>.json`.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::fs::ProjectFile;
use super::intelligence::{ProjectIntent, RelevantFile};
use super::projects::Project;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    /// List/inspect the project tree (auto-runnable).
    Inspect,
    /// Rank and read relevant files (auto-runnable).
    Context,
    /// Modify an existing file (MANUAL in Phase 1 — Phase 2 Coder executes).
    Edit,
    /// Create a new file (MANUAL in Phase 1).
    Create,
    /// Run a shell command, usually tests (auto-runnable with approval).
    Test,
    /// Verifier checklist + security review (MANUAL in Phase 1).
    Review,
    /// Anything else needing a human/agent (never auto-executed).
    Manual,
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Inspect => "inspect",
            Self::Context => "context",
            Self::Edit => "edit",
            Self::Create => "create",
            Self::Test => "test",
            Self::Review => "review",
            Self::Manual => "manual",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub kind: StepKind,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub cmd: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanGraph {
    pub version: u32,
    pub goal: String,
    pub intent: String,
    pub project_id: String,
    pub project_name: String,
    pub created_at: String,
    pub steps: Vec<PlanStep>,
}

/// Guess a test command from project markers. Shared with the debug loop.
pub fn detect_test_cmd(files: &[ProjectFile]) -> Option<String> {
    let has = |name: &str| files.iter().any(|f| f.path == name || f.path.ends_with(&format!("/{}", name)));
    let has_ext = |ext: &str| files.iter().any(|f| f.path.ends_with(ext));
    if has("Cargo.toml") {
        return Some("cargo test".to_string());
    }
    if has("package.json") {
        return Some("npm test".to_string());
    }
    if has("pytest.ini") || has("pyproject.toml") || has("setup.py") || has("requirements.txt") {
        return Some("pytest".to_string());
    }
    if has("Makefile") {
        return Some("make test".to_string());
    }
    if has_ext("_test.go") {
        return Some("go test ./...".to_string());
    }
    None
}

/// Build a deterministic execution graph for `goal`.
///
/// `relevant` should be the top-ranked files for the goal (6–8 is plenty).
/// Step ids are stable (`inspect`, `context`, `edit-1..n`, `test`, `review`)
/// so agents and transcripts can reference them.
pub fn build_plan(
    goal: &str,
    project: &Project,
    files: &[ProjectFile],
    intent: &ProjectIntent,
    relevant: &[RelevantFile],
) -> PlanGraph {
    let mut steps: Vec<PlanStep> = Vec::new();
    steps.push(PlanStep {
        id: "inspect".to_string(),
        title: "Inspect project tree".to_string(),
        kind: StepKind::Inspect,
        files: vec![],
        cmd: None,
        depends_on: vec![],
        notes: format!("List all files ({} total) to confirm scope.", files.len()),
    });

    let rel_paths: Vec<String> =
        relevant.iter().take(8).map(|r| r.file.path.clone()).collect();
    let context_notes = if rel_paths.is_empty() {
        "No files ranked relevant — broaden the goal or check the project path.".to_string()
    } else {
        format!("Top {} relevant files for this goal.", rel_paths.len())
    };
    steps.push(PlanStep {
        id: "context".to_string(),
        title: "Load relevant files".to_string(),
        kind: StepKind::Context,
        files: rel_paths.clone(),
        cmd: None,
        depends_on: vec!["inspect".to_string()],
        notes: context_notes,
    });

    // Intent-specific middle steps.
    let mut tail_depends = vec!["context".to_string()];
    match intent {
        ProjectIntent::Edit | ProjectIntent::Debug | ProjectIntent::Review => {
            // One edit candidate per relevant file (top 5); agent fills SEARCH/REPLACE.
            for (i, path) in rel_paths.iter().take(5).enumerate() {
                let id = format!("edit-{}", i + 1);
                steps.push(PlanStep {
                    id: id.clone(),
                    title: format!("Modify {}", path),
                    kind: StepKind::Edit,
                    files: vec![path.clone()],
                    cmd: None,
                    depends_on: vec!["context".to_string()],
                    notes: "MANUAL (Phase 1): propose <EDIT> with exact SEARCH anchor, or let the Phase 2 Coder execute.".to_string(),
                });
                tail_depends = vec![id];
            }
            if rel_paths.is_empty() {
                steps.push(PlanStep {
                    id: "design".to_string(),
                    title: "Design new/changed files".to_string(),
                    kind: StepKind::Manual,
                    files: vec![],
                    cmd: None,
                    depends_on: vec!["context".to_string()],
                    notes: "MANUAL: no existing files ranked — sketch the new files, then create them.".to_string(),
                });
                tail_depends = vec!["design".to_string()];
            }
        }
        ProjectIntent::Run => {
            let cmd = detect_test_cmd(files);
            steps.push(PlanStep {
                id: "run".to_string(),
                title: "Run project command".to_string(),
                kind: StepKind::Test,
                files: vec![],
                cmd,
                depends_on: vec!["context".to_string()],
                notes: "Executes with approval (exec-plan asks unless --yes).".to_string(),
            });
            tail_depends = vec!["run".to_string()];
        }
        // Explain / Find / Architecture / General / Unknown: answer from context.
        _ => {
            steps.push(PlanStep {
                id: "answer".to_string(),
                title: "Answer with citations".to_string(),
                kind: StepKind::Manual,
                files: rel_paths.clone(),
                cmd: None,
                depends_on: vec!["context".to_string()],
                notes: "Cite [path:line] for every claim; say \"Not in project context\" when unsure.".to_string(),
            });
            tail_depends = vec!["answer".to_string()];
        }
    }

    // Action plans (edit/debug/review/run) close with test + review.
    match intent {
        ProjectIntent::Edit | ProjectIntent::Debug | ProjectIntent::Review | ProjectIntent::Run => {
            if matches!(intent, ProjectIntent::Debug) {
                // Debug plans reproduce first: test cmd doubles as reproducer.
                // Insert right after `context` so edits can depend on it.
                let cmd = detect_test_cmd(files);
                let reproduce = PlanStep {
                    id: "reproduce".to_string(),
                    title: "Reproduce the failure".to_string(),
                    kind: StepKind::Test,
                    files: vec![],
                    cmd,
                    depends_on: vec!["context".to_string()],
                    notes: "Run before editing to capture the failing output.".to_string(),
                };
                let pos = steps.iter().position(|s| s.id == "context").map(|i| i + 1).unwrap_or(steps.len());
                steps.insert(pos, reproduce);
                // Edits depend on reproduction so the error is known first.
                for s in steps.iter_mut().filter(|s| s.kind == StepKind::Edit) {
                    s.depends_on = vec!["reproduce".to_string()];
                }
                tail_depends = vec!["reproduce".to_string()];
                if steps.iter().any(|s| s.kind == StepKind::Edit) {
                    tail_depends = steps
                        .iter()
                        .filter(|s| s.kind == StepKind::Edit)
                        .map(|s| s.id.clone())
                        .collect();
                }
            }
            let test_cmd = detect_test_cmd(files);
            steps.push(PlanStep {
                id: "test".to_string(),
                title: "Run tests".to_string(),
                kind: StepKind::Test,
                files: vec![],
                cmd: test_cmd,
                depends_on: tail_depends,
                notes: "If no test command was detected, verify manually.".to_string(),
            });
            steps.push(PlanStep {
                id: "review".to_string(),
                title: "Verify + security review".to_string(),
                kind: StepKind::Review,
                files: rel_paths,
                cmd: None,
                depends_on: vec!["test".to_string()],
                notes: "Verifier: no invented files/symbols. Security: no secrets, no injection, no traversal.".to_string(),
            });
        }
        _ => {}
    }

    PlanGraph {
        version: 1,
        goal: goal.to_string(),
        intent: intent.to_string(),
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        created_at: Utc::now().to_rfc3339(),
        steps,
    }
}

/// `true` when every `depends_on` id exists and points backwards (DAG sanity).
pub fn validate_graph(plan: &PlanGraph) -> Result<()> {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    for step in &plan.steps {
        for dep in &step.depends_on {
            if !seen.contains(dep.as_str()) {
                anyhow::bail!(
                    "Step '{}' depends on unknown or later step '{}'",
                    step.id,
                    dep
                );
            }
        }
        if !seen.insert(step.id.as_str()) {
            anyhow::bail!("Duplicate step id '{}'", step.id);
        }
    }
    Ok(())
}

fn plans_dir(project: &Project) -> Result<PathBuf> {
    let base = dirs::cache_dir().context("No cache dir")?;
    let dir = base.join("local-ai").join(&project.id).join("plans");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Persist a plan; returns the file path.
pub fn save_plan(project: &Project, plan: &PlanGraph) -> Result<PathBuf> {
    validate_graph(plan)?;
    let slug: String = plan
        .goal
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|w| !w.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "plan".to_string() } else { slug };
    let ts = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let path = plans_dir(project)?.join(format!("{}-{}.json", slug, ts));
    fs::write(&path, serde_json::to_string_pretty(plan)?)?;
    Ok(path)
}

pub fn load_plan(path: &std::path::Path) -> Result<PlanGraph> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Cannot read plan {}", path.display()))?;
    let plan: PlanGraph =
        serde_json::from_str(&content).context("Plan is not valid plan-graph JSON")?;
    validate_graph(&plan)?;
    Ok(plan)
}

/// Run/report persistence next to plans: `plans/runs/<plan>-<ts>.json`.
pub fn save_run_report(
    project: &Project,
    plan_path: &std::path::Path,
    report: &serde_json::Value,
) -> Result<PathBuf> {
    let dir = plans_dir(project)?.join("runs");
    fs::create_dir_all(&dir)?;
    let stem = plan_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "plan".to_string());
    let ts = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(format!("{}-run-{}.json", stem, ts));
    fs::write(&path, serde_json::to_string_pretty(report)?)?;
    Ok(path)
}
