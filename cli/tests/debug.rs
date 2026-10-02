//! Debug-loop tests — failure parsers (offline) + edit engine on temp projects.

use local_ai::core::debug as core_debug;
use local_ai::core::edits as core_edits;
use local_ai::core::projects::Project;

// ---------------------------------------------------------------------------
// parsers
// ---------------------------------------------------------------------------

#[test]
fn test_parse_rust_panic() {
    let out = "thread 'tests::auth' panicked at src/auth.rs:42:5:\nassertion failed: token.is_some()\nnote: run with RUST_BACKTRACE=1";
    let f = core_debug::parse_failures(out);
    assert!(!f.is_empty());
    let p = f.iter().find(|x| x.kind == "panic").expect("panic parsed");
    assert_eq!(p.file.as_deref(), Some("src/auth.rs"));
    assert_eq!(p.line, Some(42));
    assert!(p.message.contains("panicked") || p.message.contains("assertion"));
}

#[test]
fn test_parse_rustc_error() {
    let out = "error[E0308]: mismatched types\n --> src/main.rs:12:5\n  |\n12 |     foo(\"x\")\n   |         ^^^\n";
    let f = core_debug::parse_failures(out);
    let c = f.iter().find(|x| x.kind == "compile").expect("compile parsed");
    assert_eq!(c.file.as_deref(), Some("src/main.rs"));
    assert_eq!(c.line, Some(12));
    assert!(c.message.contains("E0308"));
}

#[test]
fn test_parse_pytest_traceback() {
    let out = "Traceback (most recent call last):\n  File \"/app/tests/test_auth.py\", line 18, in test_login\n    assert resp.status == 200\nE   AssertionError: assert 401 == 200\nFAILED tests/test_auth.py::test_login";
    let f = core_debug::parse_failures(out);
    let t = f.iter().find(|x| x.kind == "traceback").expect("traceback parsed");
    assert_eq!(t.file.as_deref(), Some("/app/tests/test_auth.py"));
    assert_eq!(t.line, Some(18));
    assert!(t.message.contains("AssertionError") || t.message.contains("FAILED"));
}

#[test]
fn test_parse_jest_stack() {
    let out = "FAIL src/auth.test.ts\n  ● login › rejects bad password\n    expect(received).toBe(expected)\n      at Object.<anonymous> (/repo/src/auth.test.ts:27:15)\n      at processTicksAndRejections (node:internal/process/task_queues:95:5)";
    let f = core_debug::parse_failures(out);
    let s = f.iter().find(|x| x.kind == "stack").expect("stack parsed");
    assert_eq!(s.file.as_deref(), Some("/repo/src/auth.test.ts"));
    assert_eq!(s.line, Some(27));
    // node: internals are skipped
    assert!(!f.iter().any(|x| x.file.as_deref().unwrap_or("").starts_with("node:")));
}

#[test]
fn test_parse_empty_and_noise() {
    assert!(core_debug::parse_failures("").is_empty());
    assert!(core_debug::parse_failures("test result: ok. 7 passed\nfinished in 0.07s").is_empty());
    // Bare FAILED header still yields a failure entry.
    let f = core_debug::parse_failures("FAILED\n");
    assert!(f.iter().any(|x| x.kind == "test"));
}

#[test]
fn test_tail_lines() {
    let out = "a\nb\nc\nd";
    assert_eq!(core_debug::tail_lines(out, 2), "c\nd");
    assert_eq!(core_debug::tail_lines(out, 99), out);
}

// ---------------------------------------------------------------------------
// edit engine
// ---------------------------------------------------------------------------

fn temp_project(tag: &str) -> (Project, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("local-ai-edits-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rs"), "fn main() {\n    todo!()\n}\n").unwrap();
    let proj = Project {
        id: format!("test-edits-{}", tag),
        name: "edits-fixture".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: Vec::new(),
    };
    (proj, dir)
}

#[test]
fn test_parse_edit_blocks_full() {
    let s = "<EDIT>FILE: src/a.rs SEARCH: foo() REPLACE: bar() </EDIT>\n<CREATE_FILE>FILE: src/new.rs CONTENT: fn x(){} </CREATE_FILE>\n<DELETE>FILE: src/old.rs </DELETE>";
    let ops = core_edits::parse_edit_blocks(s);
    assert_eq!(ops.len(), 3);
    assert!(matches!(&ops[0], core_edits::EditOp::Edit { path, search, replace } if path == "src/a.rs" && search == "foo()" && replace == "bar()"));
    assert!(matches!(&ops[1], core_edits::EditOp::Create { path, .. } if path == "src/new.rs"));
    assert!(matches!(&ops[2], core_edits::EditOp::Delete { path } if path == "src/old.rs"));
}

#[test]
fn test_apply_edit_search_verified() {
    let (proj, dir) = temp_project("edit-ok");
    let ops = core_edits::parse_edit_blocks("<EDIT>FILE: main.rs SEARCH: todo!() REPLACE: println!(\"hi\") </EDIT>");
    let rep = core_edits::apply_ops(&proj, &ops, false).unwrap();
    assert_eq!(rep.applied.len(), 1);
    assert!(rep.skipped.is_empty());
    let content = std::fs::read_to_string(dir.join("main.rs")).unwrap();
    assert!(content.contains("println!(\"hi\")"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_apply_bad_search_and_invented_paths_refused() {
    let (proj, dir) = temp_project("refuse");
    let ops = core_edits::parse_edit_blocks(
        "<EDIT>FILE: main.rs SEARCH: does-not-exist-xyz REPLACE: x </EDIT>\n<EDIT>FILE: src/invented.rs SEARCH: a REPLACE: b </EDIT>\n<DELETE>FILE: ../escape.rs </DELETE>",
    );
    let rep = core_edits::apply_ops(&proj, &ops, false).unwrap();
    assert!(rep.applied.is_empty());
    assert_eq!(rep.skipped.len(), 3);
    // Original untouched, nothing escaped the project dir.
    assert!(std::fs::read_to_string(dir.join("main.rs")).unwrap().contains("todo!()"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_apply_create_refuses_overwrite_and_dry_run_changes_nothing() {
    let (proj, dir) = temp_project("create");
    // Overwrite refused.
    let ops = core_edits::parse_edit_blocks("<CREATE_FILE>FILE: main.rs CONTENT: x </CREATE_FILE>");
    let rep = core_edits::apply_ops(&proj, &ops, false).unwrap();
    assert!(rep.applied.is_empty());
    // New file via dry-run changes nothing.
    let ops2 = core_edits::parse_edit_blocks("<CREATE_FILE>FILE: new.rs CONTENT: fn new(){} </CREATE_FILE>");
    let rep2 = core_edits::apply_ops(&proj, &ops2, true).unwrap();
    assert_eq!(rep2.applied.len(), 1);
    assert!(!dir.join("new.rs").exists());
    // For real it lands.
    let rep3 = core_edits::apply_ops(&proj, &ops2, false).unwrap();
    assert_eq!(rep3.applied.len(), 1);
    assert!(dir.join("new.rs").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
