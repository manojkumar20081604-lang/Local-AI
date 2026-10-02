//! Agent system — orchestrated specialists (Phase 2).
//!
//! Replaces single-LLM calls with bounded, foreground, user-invoked roles:
//!
//! ```text
//! User → Planner → {Researcher, Coder, Tester, Debugger, Reviewer} → Verifier → User
//! ```
//!
//! Every role maps to an existing primitive so nothing is reinvented:
//!
//! - Planner → [`crate::core::plan::build_plan`] (Phase 1.4 graph)
//! - Researcher → retrieval ([`crate::core::intelligence`]) + `search` + `web_fetch` (read-only)
//! - Coder → [`crate::core::edits`] `<CREATE_FILE>`/`<EDIT>` with `SEARCH` verification (build only)
//! - Tester/Debugger → [`crate::core::debug::parse_failures`] loop (Phase 1.2)
//! - Reviewer → [`crate::core::verifier`] + security checklist (secrets, injection, traversal)
//!
//! Safety (kill-switch) lives here so every caller inherits it:
//! - [`Budget`] caps steps, tool calls and wall time.
//! - [`is_dangerous_command`] refuses `rm -rf /`, `mkfs`, exfil (`curl POST`
//!   with file body) unless the run was started with `--approve dangerous`.
//! - All agent I/O is appended to
//!   `~/.cache/local-ai/<project-id>/missions/<ts>/trace.jsonl`
//!   (feeds Phase 4 mission UI + Phase 5 datasets).
//!
//! Explicit non-goal: no background daemon. All runs are foreground,
//! bounded and user-invoked (see `local-ai agent run`).

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use super::edits::EditOp;
use super::fs::ProjectFile;
use super::projects::Project;

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

/// Specialist roles. Each maps to existing primitives (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    Planner,
    Researcher,
    Coder,
    Tester,
    Debugger,
    Reviewer,
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Planner => "planner",
            Self::Researcher => "researcher",
            Self::Coder => "coder",
            Self::Tester => "tester",
            Self::Debugger => "debugger",
            Self::Reviewer => "reviewer",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for AgentRole {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "planner" => Ok(Self::Planner),
            "researcher" => Ok(Self::Researcher),
            "coder" => Ok(Self::Coder),
            "tester" => Ok(Self::Tester),
            "debugger" => Ok(Self::Debugger),
            "reviewer" => Ok(Self::Reviewer),
            _ => Err(format!(
                "unknown role '{}', expected planner|researcher|coder|tester|debugger|reviewer",
                s
            )),
        }
    }
}

/// One specialist: prompt + allowed tools + model hint for the router (Phase 3.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub role: AgentRole,
    /// System prompt fragment for this role (prepended to the project context).
    pub system_prompt: String,
    /// Tool names this role may call (subset of `project_tools()` + exec/test).
    pub tools_allowed: Vec<String>,
    /// Suggested model class (`coding`, `reason`, `simple`, `embed`).
    /// Used by the future router; today it is advisory only.
    pub model_hint: String,
}

fn agent(role: AgentRole, prompt: &str, tools: &[&str], hint: &str) -> Agent {
    Agent {
        role,
        system_prompt: prompt.to_string(),
        tools_allowed: tools.iter().map(|s| s.to_string()).collect(),
        model_hint: hint.to_string(),
    }
}

/// The six default specialists. Prompts are short on purpose — the project
/// context + verifier carry the grounding, not the prompt length.
pub fn default_agents() -> Vec<Agent> {
    vec![
        agent(
            AgentRole::Planner,
            "You are the Planner. Emit a minimal step list (inspect → context → edit → test → review). Reference ONLY real project files. No code, no edits — plan only.",
            &["list_project_files", "read_project_file", "search_project"],
            "reason",
        ),
        agent(
            AgentRole::Researcher,
            "You are the Researcher (read-only). Find relevant files/symbols via retrieval + search + web_fetch. NEVER write, exec builds, or propose edits — cite [path:line] for every claim.",
            &["list_project_files", "read_project_file", "search_project", "web_fetch"],
            "embed",
        ),
        agent(
            AgentRole::Coder,
            "You are the Coder. Return ONLY machine-readable <CREATE_FILE>/<EDIT> blocks for the MINIMAL fix. Relative paths that already exist (never invent), SEARCH must match byte-for-byte, smallest change, no refactoring, no new dependencies.",
            &["read_project_file", "search_project"],
            "coding",
        ),
        agent(
            AgentRole::Tester,
            "You are the Tester. Run the project's test command, parse failures (file:line), report pass/fail with the last 30 lines. Never edit files.",
            &["exec"],
            "simple",
        ),
        agent(
            AgentRole::Debugger,
            "You are the Debugger. Reproduce first (run tests), parse failures with file:line, rank relevant files, propose the minimal SEARCH-verified fix. One hypothesis at a time.",
            &["exec", "read_project_file", "search_project"],
            "coding",
        ),
        agent(
            AgentRole::Reviewer,
            "You are the Reviewer. Check: no invented files/symbols (verifier), no secrets (.env/.pem/AKIA), no injection (eval/innerHTML/shell), no traversal (../, absolute). Approve or list blocking issues.",
            &["read_project_file", "search_project"],
            "reason",
        ),
    ]
}

/// Tool allow-list for a role (convenience for the orchestrator + tests).
pub fn tools_for_role(role: AgentRole) -> Vec<String> {
    default_agents()
        .into_iter()
        .find(|a| a.role == role)
        .map(|a| a.tools_allowed)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Budget + approval gate (kill-switch inputs)
// ---------------------------------------------------------------------------

/// Hard bounds for one `agent run`. Exceeded → the run stops, trace records why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    /// Max plan steps / agent turns to execute.
    pub max_steps: u32,
    /// Max tool-equivalent calls (file reads + test execs + LLM calls).
    pub max_tool_calls: u32,
    /// Max wall-clock seconds for the whole run.
    pub max_wall_secs: u64,
    /// `--approve dangerous` was passed: allow denylisted commands.
    pub approve_dangerous: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_steps: 20,
            max_tool_calls: 50,
            max_wall_secs: 600,
            approve_dangerous: false,
        }
    }
}

/// When the orchestrator must ask before acting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalGate {
    /// Ask for every write/exec (default interactive behaviour).
    #[default]
    Always,
    /// Only dangerous commands need approval (plus `--yes` skips the rest).
    DangerousOnly,
    /// Never ask (non-interactive `--yes`; dangerous still needs `--approve`).
    Auto,
}

// ---------------------------------------------------------------------------
// Orchestrator state
// ---------------------------------------------------------------------------

/// Foreground run state: plan + budget + counters. No threads, no daemon.
pub struct Orchestrator {
    pub mission_id: String,
    pub budget: Budget,
    pub approval: ApprovalGate,
    pub steps_used: u32,
    pub tool_calls: u32,
    pub started: Instant,
}

impl Orchestrator {
    pub fn new(mission_id: String, budget: Budget, approval: ApprovalGate) -> Self {
        Self {
            mission_id,
            budget,
            approval,
            steps_used: 0,
            tool_calls: 0,
            started: Instant::now(),
        }
    }

    /// Fail closed when any bound is exceeded.
    pub fn check_budget(&self) -> Result<()> {
        if self.steps_used >= self.budget.max_steps {
            anyhow::bail!(
                "budget exceeded: {} steps used (max {}) — re-run with --max-steps {}",
                self.steps_used,
                self.budget.max_steps,
                self.budget.max_steps + 10
            );
        }
        if self.tool_calls >= self.budget.max_tool_calls {
            anyhow::bail!(
                "budget exceeded: {} tool calls used (max {})",
                self.tool_calls,
                self.budget.max_tool_calls
            );
        }
        if self.started.elapsed().as_secs() >= self.budget.max_wall_secs {
            anyhow::bail!(
                "budget exceeded: wall time {}s >= max {}s",
                self.started.elapsed().as_secs(),
                self.budget.max_wall_secs
            );
        }
        Ok(())
    }

    pub fn record_step(&mut self) {
        self.steps_used += 1;
    }

    pub fn record_tool_calls(&mut self, n: u32) {
        self.tool_calls += n;
    }
}

// ---------------------------------------------------------------------------
// Kill-switch: dangerous-command denylist
// ---------------------------------------------------------------------------

/// `Some(reason)` when `cmd` must never run without `--approve dangerous`.
///
/// Covers: filesystem destruction (`rm -rf /`, `mkfs`, `dd … /dev/`,
/// fork-bombs), host control (`shutdown`, `mkfs`, `diskpart`, Windows
/// `del C:\*`), shell-pipe execution (`curl … | sh`), and exfiltration
/// (`curl POST` with a file body, `cat .env | curl`, …).
pub fn is_dangerous_command(cmd: &str) -> Option<String> {
    let lower = cmd.to_lowercase();
    // Normalise whitespace runs so `rm  -rf  /` still matches.
    let norm: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");

    // 1) Filesystem destruction.
    if norm.contains("rm -rf /")
        || norm.contains("rm -fr /")
        || norm.contains("rm --recursive --force /")
        || norm == "rm -rf ~"
        || norm.contains("rm -rf ~ ")
        || norm.contains("rm -rf $home")
    {
        return Some("filesystem destruction: rm -rf against / or $HOME".to_string());
    }
    for pat in ["mkfs", "dd if=", "shred ", ":(){:|:&};:", "chmod -r 777 /", "chown -r ", "mv / /*"] {
        if norm.contains(pat) {
            return Some(format!("destructive pattern: {}", pat.trim()));
        }
    }
    // Windows destruction.
    for pat in ["diskpart", "format c:", "format d:", "del /f /s /q c:", "rmdir /s /q c:", "remove-item c:\\"] {
        if norm.contains(pat) {
            return Some(format!("destructive windows pattern: {}", pat));
        }
    }
    // Host control.
    for pat in ["shutdown", "reboot", "halt ", "poweroff", "init 0", "init 6"] {
        if norm.split_whitespace().any(|w| w == pat.trim()) || norm.contains(&format!("{} ", pat.trim())) {
            // Avoid flagging innocent mentions like "shutdown handler" in echo text:
            // require the word at a command boundary (start, after ; & |).
            let at_boundary = norm.starts_with(pat.trim())
                || norm.contains(&format!("; {}", pat.trim()))
                || norm.contains(&format!("&& {}", pat.trim()))
                || norm.contains(&format!("| {}", pat.trim()));
            if at_boundary {
                return Some(format!("host control: {}", pat.trim()));
            }
        }
    }
    // 2) Pipe-to-shell (remote code execution).
    if (norm.contains("curl ") || norm.contains("wget ")) && (norm.contains("| sh") || norm.contains("| bash") || norm.contains("| pwsh")) {
        return Some("remote code execution: curl/wget piped to shell".to_string());
    }
    // 3) Exfiltration: POST/PUT file bodies or secret files piped to network.
    let is_curlish = norm.contains("curl ") || norm.contains("wget ") || norm.contains("invoke-webrequest");
    let has_upload_flag = norm.contains("--data")
        || norm.contains(" -d ")
        || norm.contains(" -d@")
        || norm.contains("-f ")
        || norm.contains("--form")
        || norm.contains("--upload-file")
        || norm.contains(" -t ")
        || norm.contains("-x post")
        || norm.contains("-x put");
    if is_curlish && (has_upload_flag || norm.contains(" -x post") || norm.contains(" --request post")) {
        // `curl --data @file` / `-d @secret` / `-F file=@…` — file body leaving the box.
        if norm.contains('@') || norm.contains("--data-binary") {
            return Some("exfiltration: curl/wget POST with file body (@…)".to_string());
        }
        // `curl -X POST --data "token=..."` with a secret-looking literal.
        if norm.contains("token=") || norm.contains("secret") || norm.contains("api_key") || norm.contains("apikey") {
            return Some("exfiltration: curl POST with secret literal".to_string());
        }
    }
    // `cat .env | curl`, `env | curl`, `cat id_rsa | nc …`
    let has_secret_src = norm.contains(".env")
        || norm.contains(".pem")
        || norm.contains("id_rsa")
        || norm.contains("id_ed25519")
        || norm.contains("aws_secret")
        || norm.trim_start().starts_with("env ")
        || norm.contains("| env")
        || norm.contains("printenv");
    let has_net_sink = norm.contains("curl ") || norm.contains("wget ") || norm.contains("nc ") || norm.contains("ncat ") || norm.contains("ssh ");
    if has_secret_src && has_net_sink && norm.contains('|') {
        return Some("exfiltration: secret file piped to network".to_string());
    }
    None
}

/// Enforce the kill-switch. Errors when dangerous and not explicitly approved.
pub fn check_command_allowed(cmd: &str, approve_dangerous: bool) -> Result<()> {
    if let Some(reason) = is_dangerous_command(cmd) {
        if approve_dangerous {
            return Ok(());
        }
        anyhow::bail!(
            "dangerous command blocked ({}): `{}` — re-run with --approve dangerous to allow",
            reason,
            cmd.chars().take(120).collect::<String>()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Trace log (JSONL per mission)
// ---------------------------------------------------------------------------

/// One agent I/O event. Every orchestrator action appends one of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    pub ts: String,
    pub mission_id: String,
    pub agent: String,
    pub action: String,
    pub input: String,
    pub output: String,
    pub status: String,
}

impl TraceEntry {
    pub fn new(
        mission_id: &str,
        agent: &str,
        action: &str,
        input: &str,
        output: &str,
        status: &str,
    ) -> Self {
        Self {
            ts: Utc::now().to_rfc3339(),
            mission_id: mission_id.to_string(),
            agent: agent.to_string(),
            action: action.to_string(),
            input: truncate(input, 4000),
            output: truncate(output, 4000),
            status: status.to_string(),
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &s[..end])
}

fn missions_base(project: &Project) -> Result<PathBuf> {
    let base = dirs::cache_dir().context("No cache dir")?;
    let dir = base.join("local-ai").join(&project.id).join("missions");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Create `missions/<ts>-<shortid>/` and return `(mission_id, trace_path)`.
/// `trace.jsonl` is created empty so `--dry-run` previews still leave a trace.
pub fn create_mission(project: &Project) -> Result<(String, PathBuf)> {
    let mission_id = format!(
        "{}-{}",
        Utc::now().format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let dir = missions_base(project)?.join(&mission_id);
    fs::create_dir_all(&dir)?;
    let trace = dir.join("trace.jsonl");
    fs::write(&trace, "")?;
    Ok((mission_id, trace))
}

/// Append one JSON line to the mission trace (best-effort creation of parents).
pub fn append_trace(trace_path: &std::path::Path, entry: &TraceEntry) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = trace_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path)?;
    writeln!(f, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reviewer: verifier + security checklist
// ---------------------------------------------------------------------------

/// Security + grounding verdict for a proposed change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewReport {
    pub passed: bool,
    pub invented_files: Vec<String>,
    pub secrets_found: Vec<String>,
    pub injection_risks: Vec<String>,
    pub traversal_attempts: Vec<String>,
    pub notes: Vec<String>,
}

/// Scan `ops` payloads + free `text` for secrets, injection and traversal.
/// Pure and offline — the command layer merges this with `verifier.rs`.
pub fn security_review(ops: &[EditOp], text: &str) -> ReviewReport {
    let mut secrets = Vec::new();
    let mut injections = Vec::new();
    let mut traversals = Vec::new();
    let mut notes = Vec::new();

    // --- traversal: absolute or `..` targets (belt + suspenders; fs/edits already refuse) ---
    for op in ops {
        let p = op.path();
        if p.starts_with('/') || p.contains("..") || p.contains('\\') && p.contains("..") {
            traversals.push(p.to_string());
        }
        // Windows absolute (`C:\…`, `C:/…`).
        if p.len() >= 3 && p.chars().nth(1) == Some(':') {
            traversals.push(p.to_string());
        }
    }

    // --- secrets: payload contents only (never persist .env bodies, only hints) ---
    let bodies: Vec<&str> = ops
        .iter()
        .map(|op| match op {
            EditOp::Create { content, .. } => content.as_str(),
            EditOp::Edit { replace, .. } => replace.as_str(),
            EditOp::Delete { .. } => "",
        })
        .chain(std::iter::once(text))
        .collect();
    let joined = bodies.join("\n");
    let lower = joined.to_lowercase();
    let secret_markers = [
        ("AKIA", "possible AWS access key (AKIA…)"),
        ("aws_secret", "possible AWS secret"),
        ("-----begin private key", "private key material"),
        ("-----begin rsa private key", "private key material"),
        (".pem", ".pem reference — ensure only a path, never key contents"),
        ("api_key", "api_key literal — prefer env var"),
        ("apikey", "apikey literal — prefer env var"),
    ];
    for (needle, label) in secret_markers {
        if lower.contains(needle) {
            secrets.push(label.to_string());
        }
    }
    // `password = "value"` / `secret = 'value'` with a non-empty literal.
    for line in joined.lines().take(200) {
        let l = line.to_lowercase();
        if (l.contains("password") || l.contains("passwd") || l.trim_start().starts_with("secret"))
            && (l.contains('=') || l.contains(':'))
            && (l.contains('"') || l.contains('\''))
        {
            secrets.push(format!(
                "credential literal: {}",
                line.trim().chars().take(60).collect::<String>()
            ));
            break;
        }
    }

    // --- injection: risky sinks in the payload ---
    let sinks = [
        ("eval(", "eval()"),
        ("innerhtml", "innerHTML assignment"),
        ("dangerouslysetinnerhtml", "dangerouslySetInnerHTML"),
        ("shell=true", "shell=True subprocess"),
        ("os.system(", "os.system()"),
        ("document.write(", "document.write()"),
    ];
    for (needle, label) in sinks {
        if lower.contains(needle) {
            injections.push(label.to_string());
        }
    }

    if secrets.is_empty() && injections.is_empty() && traversals.is_empty() {
        notes.push("no secrets, injection sinks or traversal in payload".to_string());
    }

    ReviewReport {
        passed: secrets.is_empty() && traversals.is_empty(),
        invented_files: Vec::new(), // filled by the caller via verifier.rs
        secrets_found: secrets,
        injection_risks: injections,
        traversal_attempts: traversals,
        notes,
    }
}

/// Merge the lexical verifier's invented-file list into a [`ReviewReport`].
/// Any invented file fails the review (hallucinated edit target).
pub fn attach_verifier_invented(
    mut report: ReviewReport,
    project: &Project,
    files: &[ProjectFile],
    text: &str,
    grounding: &str,
) -> ReviewReport {
    let v = super::verifier::verify_response(text, project, files, grounding);
    if !v.invented_files.is_empty() {
        report.notes.push(format!(
            "verifier: {} invented file(s): {}",
            v.invented_files.len(),
            v.invented_files.join(", ")
        ));
    }
    if !v.edit_block_errors.is_empty() {
        for e in &v.edit_block_errors {
            report.notes.push(format!("edit check: {}", e));
        }
    }
    report.invented_files = v.invented_files;
    if !report.invented_files.is_empty() {
        report.passed = false;
    }
    report
}
