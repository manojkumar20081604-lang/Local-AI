//! Git intelligence tests — temp repo fixture, no network, no model needed.

use local_ai::core::{git as core_git, projects::Project};
use std::path::PathBuf;
use std::process::Command;

fn test_project(folder: &std::path::Path) -> Project {
    Project {
        id: "test-git".to_string(),
        name: "git-fixture".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        folder_path: Some(folder.to_string_lossy().to_string()),
        messages: Vec::new(),
    }
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be installed for these tests");
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Fresh repo: one commit on `main`, then an unstaged modification + untracked file.
fn fixture_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("local-ai-git-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-b", "main"]);
    git(&dir, &["config", "user.email", "test@local-ai.dev"]);
    git(&dir, &["config", "user.name", "LocalAI Test"]);
    std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "feat: initial commit"]);
    // Unstaged change + untracked file for status/diff assertions
    std::fs::write(dir.join("main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "scratch\n").unwrap();
    dir
}

fn cleanup(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_status_sees_branch_and_changes() {
    let dir = fixture_repo("status");
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    let st = core_git::status(&root).unwrap();
    assert_eq!(st.branch, "main");
    assert!(!st.is_clean());
    assert!(st.unstaged.iter().any(|c| c.path == "main.rs"), "unstaged: {:?}", st.unstaged);
    assert!(st.untracked.iter().any(|p| p == "notes.txt"), "untracked: {:?}", st.untracked);
    assert!(st.staged.is_empty());
    cleanup(&dir);
}

#[test]
fn test_diff_shows_unstaged_and_staged_split() {
    let dir = fixture_repo("diff");
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    let unstaged = core_git::diff(&root, false, None).unwrap();
    assert!(unstaged.contains("println!"), "diff should show edit:\n{}", unstaged);
    let staged = core_git::diff(&root, true, None).unwrap();
    assert!(staged.trim().is_empty(), "nothing staged yet");
    let file_diff = core_git::diff(&root, false, Some("main.rs")).unwrap();
    assert!(file_diff.contains("println!"));
    cleanup(&dir);
}

#[test]
fn test_log_lists_commit() {
    let dir = fixture_repo("log");
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    let commits = core_git::log(&root, 10).unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].message, "feat: initial commit");
    assert_eq!(commits[0].author, "LocalAI Test");
    assert_eq!(commits[0].short.len(), 7);
    cleanup(&dir);
}

#[test]
fn test_blame_attributes_committed_lines() {
    let dir = fixture_repo("blame");
    // Commit the current main.rs so blame has lines to attribute
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "feat: second"]);
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    let out = core_git::blame(&root, "main.rs", None).unwrap();
    assert!(out.contains("LocalAI Test"), "blame:\n{}", out);
    let ranged = core_git::blame(&root, "main.rs", Some("1,2")).unwrap();
    assert!(!ranged.trim().is_empty());
    // Bad range + escape attempts are rejected, not passed to git
    assert!(core_git::blame(&root, "main.rs", Some("abc")).is_err());
    assert!(core_git::blame(&root, "../escape.rs", None).is_err());
    assert!(core_git::blame(&root, "/etc/hosts", None).is_err());
    cleanup(&dir);
}

#[test]
fn test_branches_and_stash() {
    let dir = fixture_repo("branches");
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    let branches = core_git::branches(&root).unwrap();
    assert!(branches.iter().any(|b| b.name == "main" && b.current), "branches: {:?}", branches.iter().map(|b| &b.name).collect::<Vec<_>>());
    let stash = core_git::stash_list(&root).unwrap();
    assert!(stash.is_empty());
    cleanup(&dir);
}

#[test]
fn test_non_repo_errors_clearly() {
    let dir = std::env::temp_dir().join(format!("local-ai-git-norepo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    let err = core_git::status(&root).unwrap_err().to_string();
    assert!(err.contains("Not a git repository"), "got: {}", err);
    cleanup(&dir);
}

#[test]
fn test_recent_summary_compacts() {
    let dir = fixture_repo("summary");
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    let s = core_git::recent_summary(&root).unwrap();
    assert!(s.contains("branch: main"), "summary:\n{}", s);
    assert!(s.contains("feat: initial commit"), "summary:\n{}", s);
    assert!(s.contains("main.rs"), "summary:\n{}", s);
    cleanup(&dir);
}

// ---------------------------------------------------------------------------
// Phase 1.3 — commits
// ---------------------------------------------------------------------------

#[test]
fn test_is_conventional() {
    assert!(core_git::is_conventional("feat(auth): add JWT login"));
    assert!(core_git::is_conventional("fix: crash on empty input"));
    assert!(core_git::is_conventional("refactor(cli)!: drop legacy flags"));
    assert!(core_git::is_conventional("test(git): cover commit flow"));
    assert!(!core_git::is_conventional("add stuff"));
    assert!(!core_git::is_conventional("feat: "));
    assert!(!core_git::is_conventional("unknown(scope): x"));
    assert!(!core_git::is_conventional("feat(): x"));
    assert!(!core_git::is_conventional(""));
}

#[test]
fn test_propose_message_shapes() {
    use local_ai::core::git::FileChange;
    let staged = |xy: &str, path: &str| FileChange { xy: xy.to_string(), path: path.to_string() };
    // New source file → feat with dir scope.
    let m = core_git::propose_message(
        &[staged("A ", "cli/src/git.rs")],
        &[local_ai::core::git::DiffStat { path: "cli/src/git.rs".into(), added: 120, deleted: 0 }],
    );
    assert!(m.starts_with("feat(cli):"), "got: {}", m);
    assert!(core_git::is_conventional(&m));
    // Docs-only → docs.
    let m = core_git::propose_message(&[staged("M ", "docs/x.md"), staged("M ", "README.md")], &[]);
    assert!(m.starts_with("docs("), "got: {}", m);
    // Deletions-only → refactor.
    let m = core_git::propose_message(&[staged("D ", "src/old.rs")], &[]);
    assert!(m.starts_with("refactor("), "got: {}", m);
    // Generated-junk dominated → chore(repo).
    let m = core_git::propose_message(
        &[staged("D ", "backend/.venv/a.py"), staged("D ", "node_modules/b.js"), staged("M ", "src/x.rs")],
        &[],
    );
    assert_eq!(m, "chore(repo): remove 3 tracked generated files");
}

#[test]
fn test_diff_stat_and_commit_round_trip() {
    let dir = fixture_repo("commit");
    let proj = test_project(&dir);
    let root = core_git::project_root(&proj).unwrap();
    // Stage the unstaged main.rs modification from the fixture.
    git(&dir, &["add", "main.rs"]);
    let stats = core_git::diff_stat_staged(&root).unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].path, "main.rs");
    assert!(stats[0].added > 0);
    // Commit it.
    let hash = core_git::commit(&root, "fix(fixture): cover commit").unwrap();
    assert_eq!(hash.len(), 40);
    let log = core_git::log(&root, 5).unwrap();
    assert_eq!(log[0].message, "fix(fixture): cover commit");
    assert!(core_git::status(&root).unwrap().staged.is_empty());
    cleanup(&dir);
}

#[test]
fn test_message_mentions_guard() {
    let changed = vec!["src/auth.rs".to_string()];
    let outside = core_git::message_mentions_outside_changeset(
        "fix(auth): patch src/auth.rs and src/invented.rs", &changed);
    assert_eq!(outside, vec!["src/invented.rs".to_string()]);
    let clean = core_git::message_mentions_outside_changeset("fix(auth): handle login", &changed);
    assert!(clean.is_empty());
}

static COMMIT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_commit_gated_in_plan_mode() {
    let _guard = COMMIT_ENV_LOCK.lock().unwrap();
    let orig = std::env::var("LOCAL_AI_MODE").ok();
    std::env::set_var("LOCAL_AI_MODE", "plan");
    assert!(local_ai::core::config::require_build_mode("git commit").is_err());
    std::env::set_var("LOCAL_AI_MODE", "build");
    assert!(local_ai::core::config::require_build_mode("git commit").is_ok());
    if let Some(v) = orig {
        std::env::set_var("LOCAL_AI_MODE", v);
    } else {
        std::env::remove_var("LOCAL_AI_MODE");
    }
}
