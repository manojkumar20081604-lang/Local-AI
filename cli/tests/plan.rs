//! Plan-graph tests — deterministic builder, DAG validity, JSON round-trip.

use local_ai::core::fs::ProjectFile;
use local_ai::core::intelligence::{ProjectIntent, RelevantFile};
use local_ai::core::plan as core_plan;
use local_ai::core::projects::Project;

fn test_project() -> Project {
    Project {
        id: "test-plan".to_string(),
        name: "plan-fixture".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        folder_path: Some(std::env::temp_dir().to_string_lossy().to_string()),
        messages: Vec::new(),
    }
}

fn fixture_files() -> Vec<ProjectFile> {
    vec![
        ProjectFile { name: "Cargo.toml".into(), path: "Cargo.toml".into(), is_directory: false },
        ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false },
        ProjectFile { name: "auth.rs".into(), path: "src/auth.rs".into(), is_directory: false },
        ProjectFile { name: "auth_test.rs".into(), path: "tests/auth_test.rs".into(), is_directory: false },
    ]
}

fn relevant(paths: &[&str]) -> Vec<RelevantFile> {
    paths
        .iter()
        .map(|p| RelevantFile {
            file: ProjectFile { name: p.to_string(), path: p.to_string(), is_directory: false },
            score: 0.9,
            reasons: vec!["test".to_string()],
        })
        .collect()
}

#[test]
fn test_edit_plan_has_full_chain() {
    let proj = test_project();
    let files = fixture_files();
    let rel = relevant(&["src/auth.rs", "src/main.rs"]);
    let g = core_plan::build_plan("add JWT auth", &proj, &files, &ProjectIntent::Edit, &rel);
    core_plan::validate_graph(&g).unwrap();
    let ids: Vec<&str> = g.steps.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(&ids[0..2], &["inspect", "context"]);
    assert!(ids.contains(&"test"));
    assert!(ids.contains(&"review"));
    assert_eq!(ids.last(), Some(&"review"));
    // test command detected from Cargo.toml
    let test = g.steps.iter().find(|s| s.id == "test").unwrap();
    assert_eq!(test.cmd.as_deref(), Some("cargo test"));
}

#[test]
fn test_debug_plan_reproduces_before_edits() {
    let proj = test_project();
    let files = fixture_files();
    let rel = relevant(&["src/auth.rs"]);
    let g = core_plan::build_plan("fix crash on login", &proj, &files, &ProjectIntent::Debug, &rel);
    core_plan::validate_graph(&g).unwrap();
    let ids: Vec<&str> = g.steps.iter().map(|s| s.id.as_str()).collect();
    let rep = ids.iter().position(|i| *i == "reproduce").unwrap();
    let edit = ids.iter().position(|i| *i == "edit-1").unwrap();
    assert!(rep < edit, "reproduce must precede edits: {:?}", ids);
    let edit_step = g.steps.iter().find(|s| s.id == "edit-1").unwrap();
    assert_eq!(edit_step.depends_on, vec!["reproduce".to_string()]);
}

#[test]
fn test_explain_plan_has_no_test_or_review() {
    let proj = test_project();
    let files = fixture_files();
    let rel = relevant(&["src/main.rs"]);
    let g = core_plan::build_plan("explain this repo", &proj, &files, &ProjectIntent::Explain, &rel);
    core_plan::validate_graph(&g).unwrap();
    assert!(g.steps.iter().any(|s| s.id == "answer"));
    assert!(!g.steps.iter().any(|s| s.id == "test"));
}

#[test]
fn test_detect_test_cmd_markers() {
    let npm = vec![ProjectFile { name: "package.json".into(), path: "package.json".into(), is_directory: false }];
    assert_eq!(core_plan::detect_test_cmd(&npm).as_deref(), Some("npm test"));
    let py = vec![ProjectFile { name: "pytest.ini".into(), path: "pytest.ini".into(), is_directory: false }];
    assert_eq!(core_plan::detect_test_cmd(&py).as_deref(), Some("pytest"));
    let empty: Vec<ProjectFile> = vec![];
    assert_eq!(core_plan::detect_test_cmd(&empty), None);
}

#[test]
fn test_validate_rejects_bad_edges() {
    let proj = test_project();
    let files = fixture_files();
    let mut g = core_plan::build_plan("x", &proj, &files, &ProjectIntent::Edit, &relevant(&["src/main.rs"]));
    g.steps[1].depends_on = vec!["nope".to_string()];
    assert!(core_plan::validate_graph(&g).is_err());
}

#[test]
fn test_json_round_trip() {
    let proj = test_project();
    let files = fixture_files();
    let g = core_plan::build_plan("add auth", &proj, &files, &ProjectIntent::Edit, &relevant(&["src/auth.rs"]));
    let json = serde_json::to_string_pretty(&g).unwrap();
    assert!(json.contains("\"goal\""));
    assert!(json.contains("\"depends_on\""));
    let back: core_plan::PlanGraph = serde_json::from_str(&json).unwrap();
    assert_eq!(back.steps.len(), g.steps.len());
    core_plan::validate_graph(&back).unwrap();
}
