//! Phase 5 - Hallucination verifier unit tests
//! Validates: invented file detection via lexical verifier (groundrails),
//! LettuceDetect-style symbol span checks, and hallucination scoring (>0.3)
//! Reference: plan.md §5 tests/hallucination.rs

use local_ai::core::fs::ProjectFile;
use local_ai::core::projects::{Project, ChatMessage};
use local_ai::core::verifier::{
    extract_file_mentions, extract_symbols, is_inventory_query, is_project_name_query,
    verify_response, verify_response_hybrid, inject_citations, deterministic_inventory_response,
};
use std::fs;
use std::path::Path;

fn make_project_with_dir(dir: &Path) -> (Project, Vec<ProjectFile>) {
    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "test-project".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: vec![],
    };
    // Create some real files on disk
    fs::create_dir_all(dir.join("src/services")).unwrap();
    fs::create_dir_all(dir.join("cli/src/core")).unwrap();
    fs::write(dir.join("src/services/ai.ts"), "export function chat() {}\n").unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() { println!(\"hello\"); }\n").unwrap();
    fs::write(dir.join("cli/src/core/fs.rs"), "pub fn list_project_files() {}\nfn getUser() {}\n").unwrap();
    fs::write(dir.join("README.md"), "# Test\n").unwrap();

    let files = vec![
        ProjectFile { name: "ai.ts".into(), path: "src/services/ai.ts".into(), is_directory: false },
        ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false },
        ProjectFile { name: "fs.rs".into(), path: "cli/src/core/fs.rs".into(), is_directory: false },
        ProjectFile { name: "README.md".into(), path: "README.md".into(), is_directory: false },
        ProjectFile { name: "services".into(), path: "src/services".into(), is_directory: true },
    ];
    (proj, files)
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("local-ai-halluc-{}-{}", name, uuid::Uuid::new_v4()));
    fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn test_extract_file_mentions_basic() {
    let text = "See src/services/ai.ts and cli/src/core/fs.rs for details. Also check README.md";
    let mentions = extract_file_mentions(text);
    assert!(mentions.contains(&"src/services/ai.ts".to_string()), "should extract ai.ts path, got {:?}", mentions);
    assert!(mentions.contains(&"cli/src/core/fs.rs".to_string()));
    assert!(mentions.contains(&"README.md".to_string()));
}

#[test]
fn test_extract_file_mentions_create_file_block() {
    let text = "<CREATE_FILE>FILE: src/new_feature.ts CONTENT: hello </CREATE_FILE>";
    let mentions = extract_file_mentions(text);
    assert!(mentions.contains(&"src/new_feature.ts".to_string()), "got {:?}", mentions);
}

#[test]
fn test_extract_file_mentions_ignores_non_paths() {
    let text = "This is just text with no files, and http://example.com should be ignored";
    let mentions = extract_file_mentions(text);
    assert!(mentions.is_empty(), "should be empty, got {:?}", mentions);
}

#[test]
fn test_verify_no_hallucination() {
    let dir = tmp_dir("no-halluc");
    let (proj, files) = make_project_with_dir(&dir);
    let response = "The logic is in src/services/ai.ts and cli/src/core/fs.rs";
    let report = verify_response(response, &proj, &files, "balanced");
    assert_eq!(report.total_mentioned, 2, "should mention 2 files");
    assert_eq!(report.invented_count, 0, "invented should be 0");
    assert!((report.hallucination_score - 0.0).abs() < 0.001);
    assert!(!report.is_hallucinated, "should not be hallucinated");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_verify_invented_file_strict_fails() {
    let dir = tmp_dir("invented-strict");
    let (proj, files) = make_project_with_dir(&dir);
    // Claim an invented file src/services/authService.ts which does NOT exist
    let response = "See File: src/services/authService.ts for auth. The function getUser() is there.";
    let report = verify_response(response, &proj, &files, "strict");
    assert!(report.invented_files.contains(&"src/services/authService.ts".to_string()) || report.invented_count > 0,
        "should detect invented authService.ts, got invented={:?} mentioned={:?}", report.invented_files, report.mentioned_files);
    assert!(report.is_hallucinated, "strict should hallucinate on invented file");
    assert!(report.hallucination_score > 0.0);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_verify_invented_file_balanced_threshold() {
    let dir = tmp_dir("balanced-threshold");
    let (proj, files) = make_project_with_dir(&dir);
    // Mention 4 files, 1 invented => score 0.25 => balanced should NOT hallucinate (threshold 0.3)
    let response = "Files: src/services/ai.ts, cli/src/core/fs.rs, src/main.rs, and src/services/authService.ts";
    let report = verify_response(response, &proj, &files, "balanced");
    assert_eq!(report.total_mentioned, 4);
    assert_eq!(report.invented_count, 1);
    assert!((report.hallucination_score - 0.25).abs() < 0.01);
    assert!(!report.is_hallucinated, "balanced 0.25 should not hallucinate, threshold 0.3");

    // Now 2 invented out of 3 => score 0.66 => should hallucinate
    let response2 = "See src/services/authService.ts, src/utils/foo.ts, and cli/src/core/fs.rs";
    let report2 = verify_response(response2, &proj, &files, "balanced");
    // Only fs.rs exists, 2 invented => score ~0.66
    assert!(report2.is_hallucinated, "balanced 0.66 should hallucinate, got score {}", report2.hallucination_score);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_verify_hallucination_score_calculation() {
    let dir = tmp_dir("score-calc");
    let (proj, files) = make_project_with_dir(&dir);
    let response = "Invented: src/a.ts, src/b.ts, real: src/main.rs";
    // src/a.ts and src/b.ts invented, src/main.rs real => 2/3 = 0.66
    let report = verify_response(response, &proj, &files, "creative");
    assert_eq!(report.total_mentioned, 3);
    assert_eq!(report.invented_count, 2);
    assert!((report.hallucination_score - 0.666).abs() < 0.01, "got {}", report.hallucination_score);
    // creative threshold 0.6 => should hallucinate
    assert!(report.is_hallucinated);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_verify_symbol_checks_lettucedetect_style() {
    let dir = tmp_dir("symbol-check");
    let (proj, files) = make_project_with_dir(&dir);
    // File cli/src/core/fs.rs contains "getUser"
    // Response claims getUser exists and also claims inventedSymbol
    let response = "Check `getUser` in cli/src/core/fs.rs and also `inventedSymbol()` in src/services/authService.ts";
    let report = verify_response(response, &proj, &files, "balanced");
    // Symbol checks should contain getUser found=true, inventedSymbol found=false
    let get_user_check = report.symbol_checks.iter().find(|s| s.symbol == "getUser");
    assert!(get_user_check.is_some(), "should have getUser symbol, got {:?}", report.symbol_checks);
    assert!(get_user_check.unwrap().found, "getUser should be found in fs.rs");

    let invented_check = report.symbol_checks.iter().find(|s| s.symbol == "inventedSymbol");
    if let Some(check) = invented_check {
        assert!(!check.found, "inventedSymbol should not be found");
    }
    // Also file authService.ts should be invented
    assert!(report.invented_files.contains(&"src/services/authService.ts".to_string()) || report.invented_count > 0);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_verify_edit_block_checks() {
    let dir = tmp_dir("edit-block");
    let (proj, files) = make_project_with_dir(&dir);
    // Valid edit: SEARCH exists in file
    let valid_edit = "<EDIT>FILE: src/main.rs SEARCH: fn main() REPLACE: fn main2() </EDIT>";
    let report_valid = verify_response(valid_edit, &proj, &files, "strict");
    assert!(report_valid.edit_block_errors.is_empty(), "valid edit should have no errors, got {:?}", report_valid.edit_block_errors);

    // Invalid edit: SEARCH not found
    let invalid_edit = "<EDIT>FILE: src/main.rs SEARCH: this_string_does_not_exist_xyz REPLACE: foo </EDIT>";
    let report_invalid = verify_response(invalid_edit, &proj, &files, "strict");
    assert!(!report_invalid.edit_block_errors.is_empty(), "invalid SEARCH should error");
    assert!(report_invalid.edit_block_errors[0].contains("SEARCH not found"));

    // Invalid file
    let invalid_file = "<EDIT>FILE: src/services/authService.ts SEARCH: foo REPLACE: bar </EDIT>";
    let report_invalid_file = verify_response(invalid_file, &proj, &files, "strict");
    assert!(report_invalid_file.edit_block_errors.iter().any(|e| e.contains("not in project")));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_groundrails_lexical_fast_path() {
    // This test mirrors stellarshenson/groundrails lexical tier: deterministic, no LLM
    // Should be <200ms per claim via pure Rust lexical (0.76 F1 baseline)
    let dir = tmp_dir("groundrails");
    let (proj, files) = make_project_with_dir(&dir);
    let start = std::time::Instant::now();
    let response = "Detailed analysis in src/services/ai.ts and src/main.rs shows flow";
    let report = verify_response(response, &proj, &files, "balanced");
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 200, "lexical verifier should be <200ms, took {:?}", elapsed);
    assert!(!report.is_hallucinated);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_hybrid_verify_with_python_fallback() {
    // Hybrid should fallback to lexical if Python not available (no shell-out hangs)
    let dir = tmp_dir("hybrid-fallback");
    let (proj, files) = make_project_with_dir(&dir);
    let response = "See src/services/ai.ts";
    // try_external true, but if Python missing, should return lexical result without error
    let report = verify_response_hybrid(response, &proj, &files, "balanced", true);
    assert_eq!(report.total_mentioned, 1);
    assert!(!report.is_hallucinated);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_inject_citations() {
    let dir = tmp_dir("citations");
    let (proj, files) = make_project_with_dir(&dir);
    let response = "Logic in src/services/ai.ts is used. See src/services/ai.ts again.";
    let report = verify_response(response, &proj, &files, "balanced");
    let cited = inject_citations(response, &report);
    assert!(cited.contains("[1]") || cited.contains("[2]"), "cited should inject [1], got {}", cited);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_inventory_router_deterministic() {
    assert!(is_inventory_query("list all files"));
    assert!(is_inventory_query("show files in project"));
    assert!(is_inventory_query("what files exist in project"));
    // "what files exist?" alone is NOT inventory (requires project/codebase suffix per verifier.rs:298)
    assert!(!is_inventory_query("what files exist?"));
    assert!(!is_inventory_query("explain auth flow"));
    assert!(is_project_name_query("what is project name?"));
    assert!(is_project_name_query("name of project?"));
}

#[test]
fn test_deterministic_inventory_response() {
    let dir = tmp_dir("inventory");
    let (proj, files) = make_project_with_dir(&dir);
    let resp = deterministic_inventory_response(&proj, &files);
    assert!(resp.contains("Total items:"), "got {}", resp);
    assert!(resp.contains("src/services/ai.ts"));
    // inventory response should have 100% precision vs files list
    let file_set: std::collections::HashSet<_> = files.iter().map(|f| f.path.as_str()).collect();
    for line in resp.lines() {
        if line.starts_with("[FILE]") {
            let path = line.trim_start_matches("[FILE] ").trim();
            assert!(file_set.contains(path), "inventory hallucinated path {}", path);
        }
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_hallucination_strict_vs_balanced_modes() {
    let dir = tmp_dir("modes");
    let (proj, files) = make_project_with_dir(&dir);
    let response = "File: src/services/authService.ts does auth";
    let strict = verify_response(response, &proj, &files, "strict");
    let balanced = verify_response(response, &proj, &files, "balanced");
    let creative = verify_response(response, &proj, &files, "creative");
    assert!(strict.is_hallucinated, "strict should hallucinate on 1 invented");
    // balanced with 1/1 = 1.0 => >0.3 => hallucinated as well (since 1 invented, score 1.0)
    assert!(balanced.is_hallucinated);
    // creative 1.0 >0.6 => hallucinated
    assert!(creative.is_hallucinated);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_verifier_python_script_exists_and_lexical_fallback() {
    // finetune/verify.py should exist and lexical path works even without Python deps
    let candidates = [
        std::path::PathBuf::from("finetune/verify.py"),
        std::path::PathBuf::from("../finetune/verify.py"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../finetune/verify.py"),
    ];
    let found = candidates.iter().any(|p| p.exists());
    assert!(found, "finetune/verify.py should exist in at least one candidate location");
}

