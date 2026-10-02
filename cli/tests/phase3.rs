//! Phase 3 tests — memory (3.1) + symbols/graph (3.2) + router (3.3).
//!
//! All offline and deterministic (no LLM, no network).

use local_ai::core::fs::ProjectFile;
use local_ai::core::projects::Project;
use std::collections::HashMap;

fn test_project(tag: &str, dir: &std::path::Path) -> Project {
    Project {
        id: format!("test-phase3-{}", tag),
        name: "phase3-fixture".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 3.1 memory
// ---------------------------------------------------------------------------

#[test]
fn test_memory_scope_parsing() {
    use local_ai::core::memory::MemoryScope;
    assert_eq!("project".parse::<MemoryScope>().unwrap(), MemoryScope::Project);
    assert_eq!("user".parse::<MemoryScope>().unwrap(), MemoryScope::User);
    assert_eq!("task".parse::<MemoryScope>().unwrap(), MemoryScope::Task);
    assert!("nope".parse::<MemoryScope>().is_err());
}

#[test]
fn test_memory_task_needs_task_id() {
    use local_ai::core::memory as mem;
    let dir = std::env::temp_dir();
    let proj = test_project("task-guard", &dir);
    assert!(mem::memory_path(mem::MemoryScope::Task, Some(&proj), None).is_err());
    assert!(mem::memory_path(mem::MemoryScope::Task, Some(&proj), Some("t1")).is_ok());
}

#[test]
fn test_memory_redacts_secrets() {
    use local_ai::core::memory::redact_secrets;
    // AWS key
    let out = redact_secrets("key AKIAIOSFODNN7EXAMPLE here");
    assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "out: {}", out);
    assert!(out.contains("[REDACTED-AWS-KEY]"));
    // PEM block
    let pem = "cert:\n-----BEGIN PRIVATE KEY-----\nabc123\ndef456\n-----END PRIVATE KEY-----\ndone";
    let out = redact_secrets(pem);
    assert!(!out.contains("abc123"), "out: {}", out);
    assert!(out.contains("[REDACTED"));
    // .env contents keep the key, drop the value
    let out = redact_secrets("DATABASE_URL=postgres://secret\nNORMAL_LINE");
    assert!(out.contains("DATABASE_URL="), "out: {}", out);
    assert!(!out.contains("postgres://secret"), "out: {}", out);
    // password literal
    let out = redact_secrets("db password = \"hunter2\"");
    assert!(!out.contains("hunter2"), "out: {}", out);
}

#[test]
fn test_memory_round_trip_and_forget() {
    use local_ai::core::memory as mem;
    let dir = std::env::temp_dir().join(format!("local-ai-mem-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let proj = test_project(&format!("roundtrip-{}", uuid::Uuid::new_v4()), &dir);

    mem::save_memory(mem::MemoryScope::Project, Some(&proj), None, "uses Rust+React").unwrap();
    let back = mem::load_memory(mem::MemoryScope::Project, Some(&proj), None).unwrap();
    assert!(back.contains("Rust"));

    mem::append_memory(mem::MemoryScope::Project, Some(&proj), None, "second line").unwrap();
    let back = mem::load_memory(mem::MemoryScope::Project, Some(&proj), None).unwrap();
    assert!(back.contains("second line"));

    // Filtered forget drops only matching lines.
    assert!(mem::forget_memory(mem::MemoryScope::Project, Some(&proj), None, Some("second")).unwrap());
    let back = mem::load_memory(mem::MemoryScope::Project, Some(&proj), None).unwrap();
    assert!(!back.contains("second line"));
    assert!(back.contains("Rust"));

    // Full forget removes the file.
    assert!(mem::forget_memory(mem::MemoryScope::Project, Some(&proj), None, None).unwrap());
    assert!(mem::load_memory(mem::MemoryScope::Project, Some(&proj), None).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_memory_never_persists_secret_contents() {
    use local_ai::core::memory as mem;
    let dir = std::env::temp_dir().join(format!("local-ai-memsec-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let proj = test_project(&format!("sec-{}", uuid::Uuid::new_v4()), &dir);
    mem::save_memory(
        mem::MemoryScope::Project,
        Some(&proj),
        None,
        "deploy key AKIAIOSFODNN7EXAMPLE in .env",
    )
    .unwrap();
    let back = mem::load_memory(mem::MemoryScope::Project, Some(&proj), None).unwrap();
    assert!(!back.contains("AKIAIOSFODNN7EXAMPLE"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_memory_merged_context_caps_and_orders() {
    use local_ai::core::memory::merged_context;
    let m = merged_context("prefers TS", "uses Rust", Some("fixed auth"));
    assert!(m.contains("USER MEMORY"));
    assert!(m.contains("PROJECT MEMORY"));
    assert!(m.contains("TASK MEMORY"));
    // Cap at ~2000 chars.
    let big = "x".repeat(5000);
    let m = merged_context(&big, &big, None);
    assert!(m.len() <= 2100, "len {}", m.len());
}

#[test]
fn test_memory_auto_update_after_commit() {
    use local_ai::core::memory as mem;
    let dir = std::env::temp_dir().join(format!("local-ai-memauto-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let proj = test_project(&format!("auto-{}", uuid::Uuid::new_v4()), &dir);
    let changed = vec!["src/auth.rs".to_string(), "Cargo.toml".to_string()];
    let path = mem::update_project_memory_after_commit(&proj, &changed, "abc1234567890").unwrap();
    assert!(path.exists());
    let back = mem::load_memory(mem::MemoryScope::Project, Some(&proj), None).unwrap();
    assert!(back.contains("abc1234"), "back: {}", back);
    assert!(back.contains("src/auth.rs"), "back: {}", back);
    assert!(back.contains("rust"), "back: {}", back);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3.2 symbols / graph
// ---------------------------------------------------------------------------

#[test]
fn test_symbols_parse_rust() {
    use local_ai::core::symbols::parse_file;
    let src = "use crate::auth::Claims;\nmod tokens;\n\npub fn login(user: &str) {}\nstruct Session { x: i32 }\nimpl Session {\n}\n";
    let fsym = parse_file("src/auth.rs", src).unwrap();
    assert_eq!(fsym.lang, "rust");
    assert!(fsym.defs.iter().any(|d| d.name == "login" && d.kind == "fn"), "{:?}", fsym.defs);
    assert!(fsym.defs.iter().any(|d| d.name == "Session"), "{:?}", fsym.defs);
    assert!(fsym.imports.iter().any(|i| i.contains("auth")), "{:?}", fsym.imports);
}

#[test]
fn test_symbols_parse_ts_and_python() {
    use local_ai::core::symbols::parse_file;
    let ts = "import { AuthService } from './services/auth';\nexport class UserController {}\nexport function login() {}\n";
    let fsym = parse_file("routes/auth.ts", ts).unwrap();
    assert!(fsym.defs.iter().any(|d| d.name == "UserController"), "{:?}", fsym.defs);
    assert!(fsym.imports.iter().any(|i| i.contains("services/auth")), "{:?}", fsym.imports);

    let py = "import os\nfrom app.db import Session\ndef login(user):\n    pass\nclass AuthService:\n    pass\n";
    let fsym = parse_file("app/auth.py", py).unwrap();
    assert!(fsym.defs.iter().any(|d| d.name == "login"), "{:?}", fsym.defs);
    assert!(fsym.defs.iter().any(|d| d.name == "AuthService"), "{:?}", fsym.defs);
    assert!(fsym.imports.iter().any(|i| i.contains("app.db")), "{:?}", fsym.imports);
}

#[test]
fn test_graph_imported_by_lists_known_importers() {
    use local_ai::core::symbols::build_graph;
    // Fixture monorepo: routes → services/auth, controller → services/auth.
    let files = vec![
        ProjectFile { name: "auth.ts".into(), path: "src/services/auth.ts".into(), is_directory: false },
        ProjectFile { name: "routes.ts".into(), path: "src/routes.ts".into(), is_directory: false },
        ProjectFile { name: "ctrl.ts".into(), path: "src/controller.ts".into(), is_directory: false },
    ];
    let contents: HashMap<&str, &str> = [
        ("src/services/auth.ts", "export class AuthService {}\nexport function auth() {}\n"),
        ("src/routes.ts", "import { AuthService } from './services/auth';\n"),
        ("src/controller.ts", "import { auth } from './services/auth';\n"),
    ]
    .into_iter()
    .collect();
    let graph = build_graph(&files, |p| contents.get(p).map(|s| s.to_string()));
    assert_eq!(graph.files.len(), 3);
    let mut by = graph.imported_by("src/services/auth.ts");
    by.sort();
    assert_eq!(by, vec!["src/controller.ts".to_string(), "src/routes.ts".to_string()]);
    // Query resolves via symbol names, not raw text.
    let hits = local_ai::core::symbols::resolve_query("where is auth handled?", &graph);
    assert!(!hits.is_empty());
    assert_eq!(hits[0].0, "src/services/auth.ts");
}

#[test]
fn test_hybrid_rank_symbol_boost() {
    use local_ai::core::intelligence::hybrid_rank_with_graph;
    use local_ai::core::symbols::build_graph;
    let files = vec![
        ProjectFile { name: "auth.rs".into(), path: "src/auth.rs".into(), is_directory: false },
        ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false },
    ];
    let contents: HashMap<&str, &str> = [
        ("src/auth.rs", "pub fn authenticate(user: &str) {}\n"),
        ("src/main.rs", "fn main() {}\n"),
    ]
    .into_iter()
    .collect();
    let graph = build_graph(&files, |p| contents.get(p).map(|s| s.to_string()));
    // No embeddings/index — keyword fallback, but the symbol boost must lift auth.rs.
    let plain = hybrid_rank_with_graph("authenticate", &files, None, None, None, None);
    let boosted = hybrid_rank_with_graph("authenticate", &files, None, None, Some(&graph), None);
    let score_of = |rs: &[local_ai::core::intelligence::RelevantFile], path: &str| {
        rs.iter().find(|r| r.file.path == path).map(|r| r.score).unwrap_or(0.0)
    };
    assert!(
        score_of(&boosted, "src/auth.rs") >= score_of(&plain, "src/auth.rs"),
        "plain={:?} boosted={:?}",
        plain,
        boosted
    );
    assert!(boosted.iter().any(|r| r.reasons.iter().any(|x| x.starts_with("symbol"))));
}

#[test]
fn test_hybrid_rank_recency_boost() {
    use local_ai::core::intelligence::hybrid_rank_with_graph;
    let files = vec![
        ProjectFile { name: "a.rs".into(), path: "src/a.rs".into(), is_directory: false },
        ProjectFile { name: "b.rs".into(), path: "src/b.rs".into(), is_directory: false },
    ];
    let mut recency = HashMap::new();
    recency.insert("src/b.rs".to_string(), 1.0);
    let ranked = hybrid_rank_with_graph("a b src", &files, None, None, None, Some(&recency));
    assert!(ranked.iter().any(|r| r.reasons.iter().any(|x| x.starts_with("recency"))));
}

// ---------------------------------------------------------------------------
// 3.3 router
// ---------------------------------------------------------------------------

#[test]
fn test_router_classify() {
    use local_ai::core::router::{classify_task, ModelClass};
    assert_eq!(classify_task("fix the crash on login"), ModelClass::Coding);
    assert_eq!(classify_task("add JWT auth"), ModelClass::Coding);
    assert_eq!(classify_task("explain the architecture"), ModelClass::Reason);
    assert_eq!(classify_task("review this code"), ModelClass::Reason);
    assert_eq!(classify_task("run the tests"), ModelClass::Simple);
    assert_eq!(classify_task("list files"), ModelClass::Simple);
}

#[test]
fn test_router_size_tags() {
    use local_ai::core::router::parse_size_billions;
    assert_eq!(parse_size_billions("qwen2.5:1.5b"), Some(1.5));
    assert_eq!(parse_size_billions("llama3.1:8b"), Some(8.0));
    assert_eq!(parse_size_billions("qwen3:32b"), Some(32.0));
    assert_eq!(parse_size_billions("Qwen/Qwen2.5-7B-Instruct"), Some(7.0));
    assert_eq!(parse_size_billions("mistral:latest"), None);
}

#[test]
fn test_router_pick_and_override() {
    use local_ai::core::router::{pick_model, ModelClass};
    let inv = vec![
        "qwen2.5:1.5b".to_string(),
        "qwen2.5:9b".to_string(),
        "qwen3:32b".to_string(),
    ];
    // simple → cheapest, coding → 9b band, reason → largest.
    assert_eq!(pick_model(ModelClass::Simple, &inv, None).as_deref(), Some("qwen2.5:1.5b"));
    assert_eq!(pick_model(ModelClass::Coding, &inv, None).as_deref(), Some("qwen2.5:9b"));
    assert_eq!(pick_model(ModelClass::Reason, &inv, None).as_deref(), Some("qwen3:32b"));
    // Config override wins (substring match).
    assert_eq!(pick_model(ModelClass::Coding, &inv, Some("32b")).as_deref(), Some("qwen3:32b"));
    // Empty inventory + override → override as-is; empty + none → None.
    assert_eq!(pick_model(ModelClass::Coding, &[], Some("custom-model")).as_deref(), Some("custom-model"));
    assert_eq!(pick_model(ModelClass::Coding, &[], None), None);
    // Fallback when no band matches: closest to 9B.
    let inv2 = vec!["tiny:1b".to_string(), "huge:70b".to_string()];
    assert_eq!(pick_model(ModelClass::Coding, &inv2, None).as_deref(), Some("tiny:1b"));
}
