//! Project conventions — `LOCAL-AI.md` (P1c).
//!
//! A `LOCAL-AI.md` file checked into the project root carries team-shared
//! rules ("Use TypeScript. Never add dependencies without asking.").
//! It outranks stored memory (which is per-machine) because it travels
//! with the repo. Contents are secrets-redacted and capped before they
//! ever reach a prompt.
//!
//! ```bash
//! local-ai chat "add auth" --project MyApp --show-context  # conventions visible at top
//! ```

use std::path::PathBuf;

use super::projects::Project;

pub const FILENAME: &str = "LOCAL-AI.md";
/// Max chars injected (rest truncated — prompts stay bounded).
pub const MAX_CHARS: usize = 4000;

fn conventions_path(project: &Project) -> Option<PathBuf> {
    let folder = project.folder_path.as_ref()?;
    let root = PathBuf::from(folder);
    if !root.is_dir() {
        return None;
    }
    Some(root.join(FILENAME))
}

/// Load + redact + cap the conventions file. `None` when absent/unreadable.
/// Read-only and plan-mode safe.
pub fn load_conventions(project: &Project) -> Option<String> {
    let path = conventions_path(project)?;
    let content = std::fs::read_to_string(&path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut capped = trimmed.to_string();
    if capped.len() > MAX_CHARS {
        let mut end = MAX_CHARS;
        while end > 0 && !capped.is_char_boundary(end) {
            end -= 1;
        }
        capped.truncate(end);
        capped.push_str("…[truncated]");
    }
    Some(super::memory::redact_secrets(&capped))
}

/// Formatted prompt block, or `None` when the project has no conventions.
pub fn conventions_block(project: &Project) -> Option<String> {
    load_conventions(project)
        .map(|c| format!("PROJECT CONVENTIONS ({} — authoritative team rules):\n{}", FILENAME, c))
}
