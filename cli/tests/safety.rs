//! P1 safety tests — diff preview, approval tiers, LOCAL-AI.md, checkpoints.
//!
//! All offline and deterministic (no LLM, no network). Git tests use temp
//! repos via the `git` CLI, mirroring `tests/git.rs`.

use local_ai::core::{edits as core_edits, git as core_git, projects::Project};
use std::path::PathBuf;
use std::process::Command;

fn test_project(tag: &str) -> (Project, PathBuf) {
    let dir = std::env::temp_dir().join(format!("local-ai-safety-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let proj = Project {
        id: format!("test-safety-{}", uuid::Uuid::new_v4()),
        name: "safety-fixture".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: Vec::new(),
    };
    (proj, dir)
}

fn cleanup(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Diff preview (P1a)
// ---------------------------------------------------------------------------

#[test]
fn test_render_diff_edit_shows_hunk() {
    let (proj, dir) = test_project("diff-edit");
    std::fs::write(dir.join("a.rs"), "line1\nold code\nline3\nline4\nline5\nline6\n").unwrap();
    let ops = vec![core_edits::EditOp::Edit {
        path: "a.rs".into(),
        search: "old code".into(),
        replace: "new code".into(),
    }];
    let diff = core_edits::render_diff(&proj, &ops);
    let text = diff.join("\n");
    assert!(text.contains("--- a/a.rs"), "{}", text);
    assert!(text.contains("-old code"), "{}", text);
    assert!(text.contains("+new code"), "{}", text);
    assert!(text.contains(" line1"), "context:\n{}", text);
    cleanup(&dir);
}

#[test]
fn test_render_diff_refusals_match_apply() {
    let (proj, dir) = test_project("diff-refuse");
    std::fs::write(dir.join("a.rs"), "hello\n").unwrap();
    let ops = vec![
        core_edits::EditOp::Edit { path: "nope.rs".into(), search: "x".into(), replace: "y".into() },
        core_edits::EditOp::Edit { path: "a.rs".into(), search: "missing anchor".into(), replace: "y".into() },
        core_edits::EditOp::Create { path: "a.rs".into(), content: "dup".into() },
        core_edits::EditOp::Delete { path: "ghost.rs".into() },
    ];
    let diff = core_edits::render_diff(&proj, &ops);
    let text = diff.join("\n");
    assert!(text.contains("! nope.rs"), "{}", text);
    assert!(text.contains("SEARCH anchor not found"), "{}", text);
    assert!(text.contains("refusing to overwrite"), "{}", text);
    assert!(text.contains("! ghost.rs"), "{}", text);
    // Same verdicts as the real apply path (preview never over-promises).
    let report = core_edits::apply_ops(&proj, &ops, true).unwrap();
    assert_eq!(report.skipped.len(), 4, "{:?}", report.skipped);
    assert!(report.applied.is_empty());
    cleanup(&dir);
}

#[test]
fn test_render_diff_create_and_delete() {
    let (proj, dir) = test_project("diff-create-del");
    std::fs::write(dir.join("gone.rs"), "bye\n").unwrap();
    let ops = vec![
        core_edits::EditOp::Create { path: "new.rs".into(), content: "fresh\n".into() },
        core_edits::EditOp::Delete { path: "gone.rs".into() },
    ];
    let text = core_edits::render_diff(&proj, &ops).join("\n");
    assert!(text.contains("+fresh"), "{}", text);
    assert!(text.contains("-bye"), "{}", text);
    cleanup(&dir);
}

// ---------------------------------------------------------------------------
// Approval tiers (P1b): session flag is process-global, never persisted
// ---------------------------------------------------------------------------

#[test]
fn test_session_approval_flag() {
    use local_ai::core::config::{approve_session, clear_session_approval, session_approved};
    clear_session_approval();
    assert!(!session_approved());
    approve_session();
    assert!(session_approved());
    clear_session_approval();
    assert!(!session_approved());
}

// ---------------------------------------------------------------------------
// LOCAL-AI.md conventions (P1c)
// ---------------------------------------------------------------------------

#[test]
fn test_conventions_missing_or_empty_is_none() {
    let (proj, dir) = test_project("conv-none");
    assert!(local_ai::core::conventions::load_conventions(&proj).is_none());
    std::fs::write(dir.join("LOCAL-AI.md"), "   \n").unwrap();
    assert!(local_ai::core::conventions::load_conventions(&proj).is_none());
    cleanup(&dir);
}

#[test]
fn test_conventions_loads_redacted_and_capped() {
    let (proj, dir) = test_project("conv-load");
    std::fs::write(dir.join("LOCAL-AI.md"), "Use TypeScript.\nNever add deps without asking.\nKey: AKIAIOSFODNN7EXAMPLE\n").unwrap();
    let loaded = local_ai::core::conventions::load_conventions(&proj).expect("loads");
    assert!(loaded.contains("TypeScript"), "{}", loaded);
    assert!(!loaded.contains("AKIAIOSFODNN7EXAMPLE"), "{}", loaded);
    let block = local_ai::core::conventions::conventions_block(&proj).expect("block");
    assert!(block.starts_with("PROJECT CONVENTIONS"), "{}", block);

    // Long files are capped, never unbounded into prompts.
    let big = "x".repeat(9000);
    std::fs::write(dir.join("LOCAL-AI.md"), big).unwrap();
    let loaded = local_ai::core::conventions::load_conventions(&proj).expect("loads");
    assert!(loaded.ends_with("…[truncated]"), "tail: {:?}", loaded.chars().rev().take(20).collect::<String>());
    assert!(loaded.len() < 5000, "len {}", loaded.len());
    cleanup(&dir);
}

// ---------------------------------------------------------------------------
// Stash checkpoints + rollback (P1d)
// ---------------------------------------------------------------------------

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be installed for these tests");
    assert!(out.status.success(), "git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
}

fn fixture_repo(tag: &str) -> (Project, PathBuf) {
    let (proj, dir) = test_project(tag);
    git(&dir, &["init", "-b", "main"]);
    git(&dir, &["config", "user.email", "test@local-ai.dev"]);
    git(&dir, &["config", "user.name", "LocalAI Test"]);
    std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "feat: initial"]);
    (proj, dir)
}

#[test]
fn test_checkpoint_stash_and_rollback_round_trip() {
    let (proj, dir) = fixture_repo("checkpoint");
    // Dirty tree: modified tracked file + untracked file.
    std::fs::write(dir.join("main.rs"), "fn main() {\n    todo!()\n}\n").unwrap();
    std::fs::write(dir.join("scratch.txt"), "wip\n").unwrap();

    let msg = core_git::checkpoint_stash(&proj, "test").expect("stash").expect("dirty → stashed");
    assert!(msg.contains("local-ai/"), "{}", msg);
    // Tree is clean afterwards; untracked scratch went into the stash too.
    let root = core_git::project_root(&proj).unwrap();
    assert!(core_git::status(&root).unwrap().is_clean());
    assert!(!dir.join("scratch.txt").exists());

    // Rollback restores everything and consumes the checkpoint.
    let out = core_git::rollback_checkpoint(&proj).expect("rollback");
    assert!(out.contains("Rolled back"), "{}", out);
    assert!(std::fs::read_to_string(dir.join("main.rs")).unwrap().contains("todo!()"));
    assert!(dir.join("scratch.txt").exists());
    assert!(
        core_git::stash_list(&root).unwrap().iter().all(|l| !l.contains("local-ai/")),
        "checkpoint consumed"
    );
    cleanup(&dir);
}

#[test]
fn test_checkpoint_skips_clean_and_non_repo() {
    let (proj, dir) = fixture_repo("checkpoint-clean");
    // Clean tree → no checkpoint.
    assert!(core_git::checkpoint_stash(&proj, "test").unwrap().is_none());
    // Rollback with no checkpoint fails closed.
    assert!(core_git::rollback_checkpoint(&proj).is_err());
    cleanup(&dir);

    // Plain directory (no git repo) → skip, never error.
    let (plain, plain_dir) = test_project("checkpoint-norepo");
    assert!(core_git::checkpoint_stash(&plain, "test").unwrap().is_none());
    cleanup(&plain_dir);
}
