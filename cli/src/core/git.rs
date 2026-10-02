//! Git intelligence — read-only git inspection for the developer agent.
//!
//! All operations are read-only (`status`, `diff`, `log`, `blame`, `branch`,
//! `stash list`) and therefore allowed in plan mode. Nothing here writes to
//! the repo — commits live behind a separate, build-gated command (Phase 1.3).
//!
//! Implemented via the `git` CLI (no extra dependencies, works wherever git
//! is installed) scoped to the project's canonicalized folder.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::projects::Project;

/// Canonical project folder. Errors if the project has no folder or it is gone.
pub fn project_root(project: &Project) -> Result<PathBuf> {
    let folder = project
        .folder_path
        .as_ref()
        .context("Project has no folder path — attach a folder first")?;
    PathBuf::from(folder)
        .canonicalize()
        .with_context(|| format!("Project folder not found: {}", folder))
}

fn run_git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("Failed to run `git` — is git installed and on PATH?")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let msg = if err.is_empty() {
            format!("exit {}", output.status.code().unwrap_or(1))
        } else {
            err
        };
        anyhow::bail!("git {} failed: {}", args.join(" "), msg);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Friendly error when the project folder is not inside a git repo.
pub fn ensure_repo(root: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(root)
        .output()
        .context("Failed to run `git` — is git installed and on PATH?")?;
    if !output.status.success() {
        anyhow::bail!(
            "Not a git repository: {} — run `git init` there first",
            root.display()
        );
    }
    Ok(())
}

/// Reject absolute paths and `..` escapes, then confine the file under `root`.
fn safe_file_arg(root: &Path, file: &str) -> Result<String> {
    let p = Path::new(file);
    if p.is_absolute() || file.contains("..") {
        anyhow::bail!("Refusing file outside project: {}", file);
    }
    let joined = root.join(p);
    // The file itself may be untracked/new (not canonicalizable); confine via parent.
    let anchor = joined
        .canonicalize()
        .unwrap_or_else(|_| {
            joined
                .parent()
                .map(|par| par.to_path_buf())
                .unwrap_or_else(|| root.to_path_buf())
        });
    let base = if anchor.is_file() {
        anchor.parent().unwrap_or(root).to_path_buf()
    } else {
        anchor
    };
    let base = base.canonicalize().unwrap_or(base);
    if !base.starts_with(root) {
        anyhow::bail!("Refusing file outside project: {}", file);
    }
    Ok(file.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Two-letter porcelain XY code, e.g. `M `, ` M`, `MM`, `A `, `R `.
    pub xy: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct GitStatus {
    pub branch: String,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<String>,
}

impl GitStatus {
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty() && self.untracked.is_empty()
    }
}

/// Parse `git status --porcelain=v1 -b`.
pub fn status(root: &Path) -> Result<GitStatus> {
    ensure_repo(root)?;
    let out = run_git(root, &["status", "--porcelain=v1", "-b"])?;
    let mut branch = String::new();
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();
    for line in out.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            // `main...origin/main` or `No commits yet on main` or `HEAD (no branch)`
            let name = header.split("...").next().unwrap_or(header);
            branch = name
                .strip_prefix("No commits yet on ")
                .unwrap_or(name)
                .to_string();
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let (xy, path) = line.split_at(2);
        let path = path.trim_start().to_string();
        // Renames look like `R  old -> new`; keep the new path.
        let path = path.split(" -> ").last().unwrap_or(&path).to_string();
        if xy == "??" {
            untracked.push(path);
            continue;
        }
        let x = xy.chars().next().unwrap_or(' ');
        let y = xy.chars().nth(1).unwrap_or(' ');
        if x != ' ' {
            staged.push(FileChange { xy: xy.to_string(), path: path.clone() });
        }
        if y != ' ' {
            unstaged.push(FileChange { xy: xy.to_string(), path });
        }
    }
    if branch.is_empty() {
        branch = run_git(root, &["branch", "--show-current"])?
            .trim()
            .to_string();
    }
    Ok(GitStatus { branch, staged, unstaged, untracked })
}

/// Unstaged (`git diff`) or staged (`git diff --staged`) diff, optionally one file.
pub fn diff(root: &Path, staged: bool, file: Option<&str>) -> Result<String> {
    ensure_repo(root)?;
    let mut args: Vec<String> = vec!["diff".to_string(), "--no-color".to_string()];
    if staged {
        args.push("--staged".to_string());
    }
    if let Some(f) = file {
        let guarded = safe_file_arg(root, f)?;
        args.push("--".to_string());
        args.push(guarded);
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_git(root, &arg_refs)
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Newest-first log. `n == 0` means `--all` up to 50.
pub fn log(root: &Path, n: u32) -> Result<Vec<Commit>> {
    ensure_repo(root)?;
    let n_str;
    let mut args: Vec<&str> = vec![
        "log",
        "--format=%H%x1f%an%x1f%ad%x1f%s",
        "--date=short",
    ];
    if n == 0 {
        args.push("-50");
    } else {
        n_str = format!("-{}", n.min(200));
        args.push(&n_str);
    }
    let out = run_git(root, &args)?;
    let mut commits = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(4, '\u{1f}');
        let (hash, author, date, message) = match (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) {
            (Some(h), Some(a), Some(d), Some(m)) => (h, a, d, m),
            _ => continue,
        };
        commits.push(Commit {
            short: hash.chars().take(7).collect(),
            hash: hash.to_string(),
            author: author.to_string(),
            date: date.to_string(),
            message: message.to_string(),
        });
    }
    Ok(commits)
}

/// `git blame` for a file, optionally restricted to `-L <range>` (e.g. `10,40`).
pub fn blame(root: &Path, file: &str, lines: Option<&str>) -> Result<String> {
    ensure_repo(root)?;
    let guarded = safe_file_arg(root, file)?;
    // Validate -L range shape early for a clear error: `N` or `N,M`.
    if let Some(range) = lines {
        let ok = {
            let mut parts = range.split(',');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(a), None, None) => a.parse::<u32>().is_ok(),
                (Some(a), Some(b), None) => a.parse::<u32>().is_ok() && b.parse::<u32>().is_ok(),
                _ => false,
            }
        };
        if !ok {
            anyhow::bail!("Invalid --lines range '{}', expected N or N,M (e.g. 10,40)", range);
        }
        run_git(root, &["blame", "-L", range, "-n", "--", &guarded])
    } else {
        run_git(root, &["blame", "-n", "--", &guarded])
    }
}

#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub current: bool,
}

pub fn branches(root: &Path) -> Result<Vec<Branch>> {
    ensure_repo(root)?;
    let out = run_git(root, &["branch", "--list", "--no-color"])?;
    let mut list = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (current, name) = match line.strip_prefix("* ") {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        // Detached HEAD shows `(HEAD detached ...)` — keep as-is, not current.
        list.push(Branch { name: name.to_string(), current });
    }
    Ok(list)
}

/// Raw `git stash list` lines (empty when no stashes).
pub fn stash_list(root: &Path) -> Result<Vec<String>> {
    ensure_repo(root)?;
    Ok(run_git(root, &["stash", "list"])?
        .lines()
        .map(|l| l.to_string())
        .filter(|l| !l.trim().is_empty())
        .collect())
}

// ---------------------------------------------------------------------------
// Commits (Phase 1.3) — writes; callers must gate on build mode + approval.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DiffStat {
    pub path: String,
    pub added: u64,
    pub deleted: u64,
}

/// Per-file insertions/deletions for the STAGED changeset (`--numstat`).
/// Binary files report `-\t-`; they are counted as 0/0 but still listed.
pub fn diff_stat_staged(root: &Path) -> Result<Vec<DiffStat>> {
    ensure_repo(root)?;
    let out = run_git(root, &["diff", "--staged", "--numstat"])?;
    let mut stats = Vec::new();
    for line in out.lines() {
        let mut parts = line.split('\t');
        let (added, deleted, path) = match (parts.next(), parts.next(), parts.next()) {
            (Some(a), Some(d), Some(p)) => (a, d, p),
            _ => continue,
        };
        // Renames: `old -> new` — attribute to the new path.
        let path = path.split(" -> ").last().unwrap_or(path).to_string();
        stats.push(DiffStat {
            path,
            added: added.parse().unwrap_or(0),
            deleted: deleted.parse().unwrap_or(0),
        });
    }
    Ok(stats)
}

const CONVENTIONAL_TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci",
    "chore", "revert",
];

/// `type[(scope)][!]: subject` with a known type and non-empty subject.
pub fn is_conventional(msg: &str) -> bool {
    let first_line = msg.lines().next().unwrap_or("").trim();
    let Some(colon) = first_line.find(':') else { return false };
    let (head, subject) = first_line.split_at(colon);
    if subject[1..].trim().is_empty() {
        return false;
    }
    let head = head.trim();
    // Breaking-change `!` attaches after `type` or after `(scope)`.
    let head = head.strip_suffix('!').unwrap_or(head);
    let ty = match head.split_once('(') {
        Some((t, rest)) => {
            let Some(s) = rest.strip_suffix(')') else { return false };
            if s.is_empty() || s.contains([' ', ')', '(']) {
                return false;
            }
            t
        }
        None => head,
    };
    !ty.is_empty() && CONVENTIONAL_TYPES.contains(&ty) && !ty.contains(' ')
}

/// Deterministic conventional message from the staged changeset.
/// Heuristic, not magic — the user/AI refines with `-m`. Shapes:
/// docs-only → `docs`, tests-only → `test`, deletions-only → `refactor`,
/// new files → `feat`, otherwise `feat` (or `chore` when only renames).
pub fn propose_message(staged: &[FileChange], stats: &[DiffStat]) -> String {
    let paths: Vec<&str> = staged.iter().map(|c| c.path.as_str()).collect();
    // Hygiene commits (Phase 0 style): dominated by venv/node_modules/backups.
    let is_generated = |p: &str| {
        let l = p.to_lowercase();
        let components: Vec<&str> = l.split('/').collect();
        let filename = components.last().copied().unwrap_or("");
        components.iter().any(|c| {
            ["venv", ".venv", "node_modules", "backup", "backups", "__pycache__"].contains(c)
        }) || filename.starts_with("test-")
    };
    if !paths.is_empty() && paths.iter().filter(|p| is_generated(p)).count() * 2 >= paths.len() {
        return format!("chore(repo): remove {} tracked generated files", paths.len());
    }
    let is_docs = |p: &str| {
        let l = p.to_lowercase();
        l.ends_with(".md") || l.ends_with(".txt") || l.contains("docs/") || l == "license"
    };
    let is_test = |p: &str| {
        let l = p.to_lowercase();
        l.contains("test") || l.contains("spec") || l.starts_with("tests/") || l.contains("/tests/")
    };
    let ty = if !paths.is_empty() && paths.iter().all(|p| is_docs(p)) {
        "docs"
    } else if !paths.is_empty() && paths.iter().all(|p| is_test(p)) {
        "test"
    } else if staged.iter().any(|c| c.xy.starts_with('A')) || staged.iter().any(|c| c.xy.starts_with('?')) {
        "feat"
    } else if !staged.is_empty() && staged.iter().all(|c| c.xy.starts_with('D')) {
        "refactor"
    } else {
        "feat"
    };
    // Scope: common top-level dir, else most-changed file stem.
    let scope = {
        let tops: Vec<&str> = paths
            .iter()
            .filter_map(|p| p.split('/').next())
            .filter(|_t| paths.iter().any(|p| p.contains('/')))
            .collect();
        let first = tops.first().copied().unwrap_or("");
        if !first.is_empty() && tops.iter().all(|t| *t == first) && first != paths[0].split('/').next_back().unwrap_or("") {
            first.to_string()
        } else {
            // Most-added file stem.
            let mut best: Option<(&str, u64)> = None;
            for s in stats {
                let stem = s.path.rsplit('/').next().unwrap_or(&s.path);
                let stem = stem.split('.').next().unwrap_or(stem);
                let weight = s.added + s.deleted;
                if best.map(|(_, w)| weight > w).unwrap_or(true) {
                    best = Some((stem, weight));
                }
            }
            best.map(|(s, _)| s.to_string()).unwrap_or_else(|| "repo".to_string())
        }
    };
    // Subject: top-3 changed stems.
    let mut subjects: Vec<(&str, u64)> = stats
        .iter()
        .map(|s| {
            let stem = s.path.rsplit('/').next().unwrap_or(&s.path);
            (stem, s.added + s.deleted)
        })
        .collect();
    subjects.sort_by_key(|a| std::cmp::Reverse(a.1));
    let shown: Vec<&str> = subjects.iter().take(3).map(|(s, _)| *s).collect();
    let subject = if shown.is_empty() {
        format!("update {} files", paths.len())
    } else if subjects.len() > 3 {
        format!("{} +{} more", shown.join(", "), subjects.len() - 3)
    } else {
        shown.join(", ")
    };
    format!("{}({}): update {}", ty, scope, subject)
}

/// Paths mentioned in a commit message that are NOT in the changeset —
/// a light hallucination guard for AI-generated messages.
pub fn message_mentions_outside_changeset(msg: &str, changed: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for token in msg.split(|c: char| c.is_whitespace() || "(),[]\"'`".contains(c)) {
        let t = token.trim().trim_matches(|c| c == '.' || c == ',' || c == ':' || c == ';');
        if t.len() < 4 || !t.contains('.') || t.contains("..") {
            continue;
        }
        let ext = t.rsplit('.').next().unwrap_or("");
        if !(1..=8).contains(&ext.len()) || !ext.chars().all(|c| c.is_alphanumeric()) {
            continue;
        }
        if !changed.iter().any(|c| c == t || c.ends_with(&format!("/{}", t))) && !out.contains(&t.to_string()) {
            out.push(t.to_string());
        }
    }
    out
}

/// Create the commit. Caller gates: build mode, approval, test-green.
/// Returns the new commit hash.
pub fn commit(root: &Path, message: &str) -> Result<String> {
    ensure_repo(root)?;
    if message.trim().is_empty() {
        anyhow::bail!("Empty commit message — pass -m \"type(scope): subject\"");
    }
    let status = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(root)
        .output()
        .context("Failed to run `git commit`")?;
    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr).trim().to_string();
        anyhow::bail!("git commit failed: {}", if err.is_empty() { "unknown error".into() } else { err });
    }
    let hash = run_git(root, &["rev-parse", "HEAD"])?.trim().to_string();
    Ok(hash)
}

/// Compact repo summary for prompt injection (`--show-context`):
/// branch + porcelain short status + last 5 oneline commits.
pub fn recent_summary(root: &Path) -> Result<String> {
    ensure_repo(root)?;
    let branch = run_git(root, &["branch", "--show-current"])?
        .trim()
        .to_string();
    let short = run_git(root, &["status", "--short", "-b"])?;
    let log_lines = run_git(root, &["log", "--oneline", "-5"])?;
    let mut out = String::new();
    out.push_str(&format!("branch: {}\n", branch));
    out.push_str("status:\n");
    out.push_str(&short);
    if !short.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("recent:\n");
    out.push_str(&log_lines);
    Ok(out)
}
