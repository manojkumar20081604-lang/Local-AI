//! Machine-readable file edits — the apply engine behind the debug loop
//! (Phase 1.2) and the future Phase 2 Coder.
//!
//! Format (same markers the chat prompt teaches the model):
//! ```text
//! <CREATE_FILE>FILE: path CONTENT: ... </CREATE_FILE>
//! <EDIT>FILE: path SEARCH: ... REPLACE: ... </EDIT>
//! <DELETE>FILE: path </DELETE>
//! ```
//!
//! Safety rules (all enforced here, so every caller inherits them):
//! - `EDIT`/`DELETE` targets must already exist in the project inventory —
//!   invented paths are skipped, never created implicitly.
//! - `EDIT` applies only when `SEARCH` matches the file byte-for-byte
//!   (after stripping one leading newline past the marker — block
//!   formatting usually puts the code on the next line).
//! - `CREATE` refuses to overwrite an existing file.
//! - Path traversal is enforced by `core::fs` on every write/delete.
//! - `dry_run` verifies everything and changes nothing.

use anyhow::Result;
use std::collections::HashSet;

use super::fs as core_fs;
use super::projects::Project;

#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    Create { path: String, content: String },
    Edit { path: String, search: String, replace: String },
    Delete { path: String },
}

impl EditOp {
    pub fn path(&self) -> &str {
        match self {
            Self::Create { path, .. } | Self::Edit { path, .. } | Self::Delete { path } => path,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Create { .. } => "CREATE",
            Self::Edit { .. } => "EDIT",
            Self::Delete { .. } => "DELETE",
        }
    }
}

/// Strip block-formatting whitespace around the payload: one run of
/// spaces/tabs/newlines after the marker, all trailing whitespace.
/// Interior content is preserved byte-for-byte. If an anchor legitimately
/// starts with indentation on the marker line itself, the op will simply
/// fail closed at SEARCH-verification time (safe refusal, never a bad edit).
fn clean_block(s: &str) -> String {
    s.trim_start_matches([' ', '\t', '\n', '\r'])
        .trim_end_matches([' ', '\t', '\n', '\r'])
        .to_string()
}

fn text_between(block: &str, start_marker: &str, end_markers: &[&str]) -> Option<String> {
    let start = block.find(start_marker)? + start_marker.len();
    let rest = &block[start..];
    let mut end = rest.len();
    for m in end_markers {
        if let Some(pos) = rest.find(m) {
            end = end.min(pos);
        }
    }
    Some(clean_block(&rest[..end]))
}

/// Extract full edit operations (with contents) from a model response,
/// in document order.
pub fn parse_edit_blocks(s: &str) -> Vec<EditOp> {
    const TAGS: &[(&str, &str)] = &[
        ("<CREATE_FILE>", "</CREATE_FILE>"),
        ("<EDIT>", "</EDIT>"),
        ("<DELETE>", "</DELETE>"),
    ];
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < s.len() {
        // Nearest opening tag at or after idx.
        let mut next: Option<(usize, usize)> = None;
        for (ti, (open, _)) in TAGS.iter().enumerate() {
            if let Some(pos) = s[idx..].find(open) {
                let abs = idx + pos;
                if next.map(|(p, _)| abs < p).unwrap_or(true) {
                    next = Some((abs, ti));
                }
            }
        }
        let Some((open_pos, ti)) = next else { break };
        let (open, close) = TAGS[ti];
        let body_start = open_pos + open.len();
        let Some(close_rel) = s[body_start..].find(close) else { break };
        let block = &s[body_start..body_start + close_rel];
        match ti {
            // CREATE_FILE — content runs to the closing tag.
            0 => {
                if let Some(path) = text_between(block, "FILE:", &["CONTENT:"]) {
                    let content = text_between(block, "CONTENT:", &[]).unwrap_or_default();
                    if !path.trim().is_empty() {
                        out.push(EditOp::Create { path: path.trim().to_string(), content });
                    }
                }
            }
            // EDIT — FILE / SEARCH / REPLACE in order.
            1 => {
                let path = text_between(block, "FILE:", &["SEARCH:"]);
                let search = text_between(block, "SEARCH:", &["REPLACE:"]);
                let replace = text_between(block, "REPLACE:", &[]);
                if let (Some(p), Some(srch), Some(r)) = (path, search, replace) {
                    if !p.trim().is_empty() && !srch.is_empty() {
                        out.push(EditOp::Edit { path: p.trim().to_string(), search: srch, replace: r });
                    }
                }
            }
            // DELETE — `FILE:` prefix optional.
            _ => {
                let raw = clean_block(block);
                let path = raw.strip_prefix("FILE:").map(|p| p.trim().to_string()).unwrap_or(raw.trim().to_string());
                if !path.is_empty() {
                    out.push(EditOp::Delete { path });
                }
            }
        }
        idx = body_start + close_rel + close.len();
    }
    out
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub applied: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// Max diff lines rendered per op (rest summarized, never silently dropped).
const DIFF_CONTEXT_LINES: usize = 3;
const DIFF_MAX_LINES_PER_OP: usize = 60;

/// Render a unified-diff-style preview of `ops` against current file
/// contents. Read-only: never writes. Refused ops render as
/// `! path — reason` with the same verdicts `apply_ops` would give,
/// so the preview never promises what apply would refuse.
pub fn render_diff(project: &Project, ops: &[EditOp]) -> Vec<String> {
    let files = core_fs::list_project_files(project).unwrap_or_default();
    let inventory: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut out = Vec::new();
    for op in ops {
        out.push(format!("--- a/{} | +++ b/{}", op.path(), op.path()));
        match op {
            EditOp::Create { path, content } => {
                if inventory.contains(path.as_str()) {
                    out.push(format!("! {} — exists, refusing to overwrite (use EDIT)", path));
                    continue;
                }
                push_capped(&mut out, content.lines().map(|l| format!("+{}", l)), "new file");
            }
            EditOp::Edit { path, search, replace } => {
                if !inventory.contains(path.as_str()) {
                    out.push(format!("! {} — not in project inventory, refusing invented path", path));
                    continue;
                }
                let current = core_fs::read_project_file(project, path).unwrap_or_default();
                let Some(anchor_at) = current.find(search.as_str()) else {
                    out.push(format!("! {} — SEARCH anchor not found, would refuse", path));
                    continue;
                };
                // Hunk window around the anchor (line-oriented, capped).
                let anchor_end = anchor_at + search.len();
                let lines: Vec<&str> = current.lines().collect();
                let first = current[..anchor_at].matches('\n').count();
                let last = current[..anchor_end.min(current.len())].matches('\n').count();
                let from = first.saturating_sub(DIFF_CONTEXT_LINES);
                let to = (last + DIFF_CONTEXT_LINES + 1).min(lines.len());
                for (i, line) in lines.iter().enumerate().take(to).skip(from) {
                    if (first..=last).contains(&i) {
                        out.push(format!("-{}", line));
                    } else {
                        out.push(format!(" {}", line));
                    }
                    if out.len() >= DIFF_MAX_LINES_PER_OP {
                        break;
                    }
                }
                for line in replace.lines().take(DIFF_MAX_LINES_PER_OP.saturating_sub(out.len())) {
                    out.push(format!("+{}", line));
                }
                if out.len() >= DIFF_MAX_LINES_PER_OP {
                    out.push("… (diff truncated)".to_string());
                }
            }
            EditOp::Delete { path } => {
                if !inventory.contains(path.as_str()) {
                    out.push(format!("! {} — not in project inventory, refusing invented path", path));
                    continue;
                }
                let current = core_fs::read_project_file(project, path).unwrap_or_default();
                push_capped(&mut out, current.lines().map(|l| format!("-{}", l)), "deleted file");
            }
        }
    }
    out
}

fn push_capped(out: &mut Vec<String>, lines: impl Iterator<Item = String>, what: &str) {
    for (n, line) in lines.enumerate() {
        if n >= DIFF_MAX_LINES_PER_OP {
            out.push(format!("… ({} truncated)", what));
            return;
        }
        out.push(line);
    }
}

/// Apply ops against the project. Never bails on a single bad op — the
/// failure is recorded in `skipped` and the rest still apply.
pub fn apply_ops(project: &Project, ops: &[EditOp], dry_run: bool) -> Result<ApplyReport> {
    let files = core_fs::list_project_files(project)?;
    let inventory: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut report = ApplyReport::default();

    for op in ops {
        let outcome: Result<String> = (|| {
            match op {
                EditOp::Create { path, content } => {
                    if inventory.contains(path.as_str()) {
                        anyhow::bail!("exists — refusing to overwrite (use EDIT)");
                    }
                    if dry_run {
                        return Ok(format!("would-create {} ({} chars)", path, content.len()));
                    }
                    core_fs::write_project_file(project, path, content)?;
                    Ok(format!("created {}", path))
                }
                EditOp::Edit { path, search, replace } => {
                    if !inventory.contains(path.as_str()) {
                        anyhow::bail!("not in project inventory — refusing invented path");
                    }
                    let current = core_fs::read_project_file(project, path)?;
                    if !current.contains(search.as_str()) {
                        anyhow::bail!("SEARCH anchor not found in {}", path);
                    }
                    if dry_run {
                        return Ok(format!("would-edit {} (anchor ok)", path));
                    }
                    let updated = current.replacen(search.as_str(), replace.as_str(), 1);
                    core_fs::write_project_file(project, path, &updated)?;
                    Ok(format!("edited {}", path))
                }
                EditOp::Delete { path } => {
                    if !inventory.contains(path.as_str()) {
                        anyhow::bail!("not in project inventory — refusing invented path");
                    }
                    if dry_run {
                        return Ok(format!("would-delete {}", path));
                    }
                    core_fs::delete_project_file(project, path)?;
                    Ok(format!("deleted {}", path))
                }
            }
        })();
        match outcome {
            Ok(msg) => report.applied.push(msg),
            Err(e) => report.skipped.push((op.path().to_string(), e.to_string())),
        }
    }
    Ok(report)
}
