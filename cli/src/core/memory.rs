//! Three-tier memory (Phase 3.1).
//!
//! ```text
//! project-memory.md  stack, arch, important files (per project)
//! user-memory.md     prefs: language, verbosity, OS, models (global)
//! task-memory/<id>.md  what changed / failed / worked last time (per task)
//! ```
//!
//! Storage (OS-appropriate, alongside existing `projects.json` / `index.json`):
//!
//! - user scope: `~/.config/local-ai/memory-user.md`
//! - project scope: `~/.cache/local-ai/<project-id>/memory.md`
//! - task scope: `~/.cache/local-ai/<project-id>/tasks/<task-id>.md`
//!
//! Secrets guard: [`redact_secrets`] strips key material before anything is
//! persisted — only paths + `[REDACTED]` hints ever hit disk. `memory set`
//! is build-gated (it writes); `memory show` is read-only and plan-mode safe.
//!
//! After a successful `git commit`, [`update_project_memory_after_commit`]
//! appends a one-line journal entry (files changed, stack detected) so the
//! next session cold-starts with context.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::projects::Project;

/// Memory tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    Project,
    User,
    Task,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Task => "task",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "project" => Ok(Self::Project),
            "user" => Ok(Self::User),
            "task" => Ok(Self::Task),
            _ => Err(format!("unknown scope '{}', expected project|user|task", s)),
        }
    }
}

fn user_memory_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("No config dir")?;
    Ok(base.join("local-ai").join("memory-user.md"))
}

fn project_memory_path(project: &Project) -> Result<PathBuf> {
    let base = dirs::cache_dir().context("No cache dir")?;
    Ok(base.join("local-ai").join(&project.id).join("memory.md"))
}

fn task_memory_path(project: &Project, task_id: &str) -> Result<PathBuf> {
    let safe: String = task_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let safe = safe.trim_matches('-').to_string();
    if safe.is_empty() {
        anyhow::bail!("Invalid --task-id '{}'", task_id);
    }
    let base = dirs::cache_dir().context("No cache dir")?;
    Ok(base
        .join("local-ai")
        .join(&project.id)
        .join("tasks")
        .join(format!("{}.md", safe)))
}

/// Resolve the file for a scope. `task_id` is required for `Task`.
pub fn memory_path(
    scope: MemoryScope,
    project: Option<&Project>,
    task_id: Option<&str>,
) -> Result<PathBuf> {
    match scope {
        MemoryScope::User => user_memory_path(),
        MemoryScope::Project => {
            let p = project.context("--scope project needs --project (or current dir)")?;
            project_memory_path(p)
        }
        MemoryScope::Task => {
            let p = project.context("--scope task needs --project")?;
            let t = task_id.context("--scope task needs --task-id <id>")?;
            task_memory_path(p, t)
        }
    }
}

/// Read raw memory text (`""` when nothing stored yet).
pub fn load_memory(
    scope: MemoryScope,
    project: Option<&Project>,
    task_id: Option<&str>,
) -> Result<String> {
    let path = memory_path(scope, project, task_id)?;
    if !path.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(&path).unwrap_or_default())
}

/// Persist memory text after [`redact_secrets`]. Creates parents as needed.
pub fn save_memory(
    scope: MemoryScope,
    project: Option<&Project>,
    task_id: Option<&str>,
    content: &str,
) -> Result<PathBuf> {
    let path = memory_path(scope, project, task_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, redact_secrets(content))?;
    Ok(path)
}

/// Append one journal line (redacted). Used by `memory set --append` and the
/// post-commit auto-update.
pub fn append_memory(
    scope: MemoryScope,
    project: Option<&Project>,
    task_id: Option<&str>,
    line: &str,
) -> Result<PathBuf> {
    let current = load_memory(scope, project, task_id)?;
    let next = if current.trim().is_empty() {
        format!("{}\n", line.trim())
    } else {
        format!("{}\n{}\n", current.trim_end(), line.trim())
    };
    save_memory(scope, project, task_id, &next)
}

/// Remove stored memory. For `forget` with a `filter`, only lines containing
/// the filter (case-insensitive) are dropped; otherwise the whole file goes.
pub fn forget_memory(
    scope: MemoryScope,
    project: Option<&Project>,
    task_id: Option<&str>,
    filter: Option<&str>,
) -> Result<bool> {
    let path = memory_path(scope, project, task_id)?;
    if !path.exists() {
        return Ok(false);
    }
    if let Some(f) = filter {
        let current = fs::read_to_string(&path).unwrap_or_default();
        let needle = f.to_lowercase();
        let kept: Vec<&str> = current
            .lines()
            .filter(|l| !l.to_lowercase().contains(&needle))
            .collect();
        if kept.len() == current.lines().count() {
            return Ok(false);
        }
        fs::write(&path, redact_secrets(&kept.join("\n")))?;
        return Ok(true);
    }
    fs::remove_file(&path)?;
    Ok(true)
}

/// Merged context block for prompt injection (user → project → task).
/// Empty tiers are skipped; output is capped to ~2000 chars.
pub fn merged_context(
    user: &str,
    project_mem: &str,
    task: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if !user.trim().is_empty() {
        parts.push(format!("USER MEMORY:\n{}", user.trim()));
    }
    if !project_mem.trim().is_empty() {
        parts.push(format!("PROJECT MEMORY:\n{}", project_mem.trim()));
    }
    if let Some(t) = task {
        if !t.trim().is_empty() {
            parts.push(format!("TASK MEMORY:\n{}", t.trim()));
        }
    }
    let mut out = parts.join("\n\n");
    if out.len() > 2000 {
        let mut end = 2000;
        while end > 0 && !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str("\n…[truncated]");
    }
    out
}

// ---------------------------------------------------------------------------
// Secrets guard
// ---------------------------------------------------------------------------

/// Redact key material so only paths + hints persist.
///
/// - `-----BEGIN … PRIVATE KEY----- … -----END …-----` blocks → placeholder
/// - `AKIA…` (20-char AWS key) → `[REDACTED-AWS-KEY]`
/// - `password|secret = "value"` literals → `[REDACTED]`
/// - `.env` file *contents* (`KEY=value` lines) → kept as `KEY=[REDACTED]`
///   so the variable name survives but the value does not.
pub fn redact_secrets(text: &str) -> String {
    let mut out = text.to_string();

    // 1) PEM blocks (may span lines) — collapse to a one-line hint.
    out = strip_pem_blocks(&out);

    // 2) AWS access keys.
    out = redact_akia(&out);

    // 3) Line-wise credential literals + .env contents.
    let mut lines = Vec::new();
    for line in out.lines() {
        lines.push(redact_line(line));
    }
    lines.join("\n")
}

fn strip_pem_blocks(s: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in s.lines() {
        let t = line.trim();
        if t.contains("BEGIN") && t.contains("PRIVATE KEY") {
            skipping = true;
            out.push_str("[REDACTED private key — path only]\n");
            continue;
        }
        if skipping {
            if (t.contains("END") && t.contains("PRIVATE KEY")) || t.is_empty() {
                if t.contains("END") {
                    skipping = false;
                }
                continue;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn redact_akia(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if s[i..].starts_with("AKIA") && s[i..].len() >= 20 {
            let candidate: String = s[i..].chars().take(20).collect();
            if candidate.len() == 20
                && candidate.chars().skip(4).all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                out.push_str("[REDACTED-AWS-KEY]");
                i += 20;
                continue;
            }
        }
        // char-boundary-safe advance
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_line(line: &str) -> String {
    let lower = line.to_lowercase();
    // .env-style `KEY=value` (no spaces around = is common) — keep the key.
    if !line.contains(' ') || line.contains('=') {
        if let Some(eq) = line.find('=') {
            let (k, _) = line.split_at(eq);
            let key = k.trim();
            if !key.is_empty()
                && !key.contains(' ')
                && (lower.contains("password")
                    || lower.contains("secret")
                    || lower.contains("token")
                    || lower.contains("api_key")
                    || lower.contains("apikey")
                    || key.to_uppercase() == key && key.len() >= 3)
                && line[eq..].len() > 1
            {
                // Only redact when there is an actual value (and quotes or length).
                let val = line[eq + 1..].trim();
                if !val.is_empty() && val != "[REDACTED]" {
                    return format!("{}=[REDACTED]", key);
                }
            }
        }
    }
    // `password : "value"` / `secret = 'value'` prose lines.
    if (lower.contains("password") || lower.contains("secret"))
        && (line.contains('"') || line.contains('\''))
        && (line.contains('=') || line.contains(':'))
    {
        // Keep up to the separator, redact the rest.
        if let Some(pos) = line.find('=') {
            return format!("{} [REDACTED]", line[..pos].trim_end());
        }
        if let Some(pos) = line.find(':') {
            // Avoid mangling `https://…` — only when the left side mentions creds.
            let left = &line[..pos];
            if left.to_lowercase().contains("password") || left.to_lowercase().contains("secret") {
                return format!("{}: [REDACTED]", left.trim_end());
            }
        }
    }
    line.to_string()
}

// ---------------------------------------------------------------------------
// Post-commit auto-update
// ---------------------------------------------------------------------------

/// Best-effort journal entry after `git commit`: date + changed files +
/// detected stack. Never fails the commit (errors are swallowed by the caller).
pub fn update_project_memory_after_commit(
    project: &Project,
    changed: &[String],
    commit_hash: &str,
) -> Result<PathBuf> {
    let stack = detect_stack(changed);
    let shown: Vec<&str> = changed.iter().take(8).map(|s| s.as_str()).collect();
    let more = if changed.len() > 8 {
        format!(" +{} more", changed.len() - 8)
    } else {
        String::new()
    };
    let line = format!(
        "- {} commit {} ({} files: {}{}; stack: {})",
        chrono::Utc::now().format("%Y-%m-%d"),
        &commit_hash[..7.min(commit_hash.len())],
        changed.len(),
        shown.join(", "),
        more,
        if stack.is_empty() { "unknown".to_string() } else { stack.join("+") }
    );
    append_memory(MemoryScope::Project, Some(project), None, &line)
}

fn detect_stack(changed: &[String]) -> Vec<String> {
    let mut stack = Vec::new();
    let has = |needle: &str| changed.iter().any(|p| p == needle || p.ends_with(&format!("/{}", needle)) || p.ends_with(needle));
    let has_ext = |ext: &str| changed.iter().any(|p| p.ends_with(ext));
    if has("Cargo.toml") || has_ext(".rs") {
        stack.push("rust".to_string());
    }
    if has("package.json") || has_ext(".ts") || has_ext(".tsx") || has_ext(".js") {
        stack.push("node".to_string());
    }
    if has("requirements.txt") || has("pyproject.toml") || has_ext(".py") {
        stack.push("python".to_string());
    }
    if has("go.mod") || has_ext(".go") {
        stack.push("go".to_string());
    }
    if has("Dockerfile") || has("docker-compose.yml") {
        stack.push("docker".to_string());
    }
    if changed.iter().any(|p| p.contains("postgres") || p.ends_with(".sql")) {
        stack.push("postgres".to_string());
    }
    stack
}
