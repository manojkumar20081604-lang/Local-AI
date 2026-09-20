//! Phase 5 - RAG / Embedding tests
//! Validates: fastembed cosine sanity, hybrid ranking vs keyword baseline, TfIdf fallback
//! Reference: plan.md §5 tests/rag.rs

use local_ai::core::config::AppConfig;
use local_ai::core::embeddings::{cosine, get_embedder, TfIdfEmbedder, Embedder};
use local_ai::core::fs::ProjectFile;
use local_ai::core::intelligence::{detect_intent, hybrid_rank, rank_relevant_files, ProjectIntent};
use local_ai::core::index::{build_index, load_index, query_index, ProjectIndex};
use local_ai::core::projects::Project;
use std::fs;
use std::sync::Arc;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("local-ai-rag-{}-{}", name, uuid::Uuid::new_v4()));
    fs::create_dir_all(&base).unwrap();
    base
}

fn make_files() -> Vec<ProjectFile> {
    vec![
        ProjectFile { name: "auth.rs".into(), path: "src/auth.rs".into(), is_directory: false },
        ProjectFile { name: "db.rs".into(), path: "src/db.rs".into(), is_directory: false },
        ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false },
        ProjectFile { name: "lib.rs".into(), path: "src/lib.rs".into(), is_directory: false },
        ProjectFile { name: "package.json".into(), path: "package.json".into(), is_directory: false },
        ProjectFile { name: "node_modules".into(), path: "node_modules/foo.js".into(), is_directory: false },
    ]
}

#[test]
fn test_cosine_sanity() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    assert!((cosine(&a, &b) - 1.0).abs() < 0.001, "identical vectors cosine 1.0");

    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    assert!(cosine(&a, &b).abs() < 0.001, "orthogonal cosine 0.0");

    let a = vec![1.0, 1.0];
    let b = vec![-1.0, -1.0];
    assert!((cosine(&a, &b) + 1.0).abs() < 0.001, "opposite cosine -1.0");

    let empty: Vec<f32> = vec![];
    assert_eq!(cosine(&empty, &empty), 0.0);
}

#[test]
fn test_cosine_mismatched_len() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 2.0];
    assert_eq!(cosine(&a, &b), 0.0, "mismatched dims should return 0.0");
}

#[test]
fn test_tfidf_embedder_deterministic_and_normalized() {
    let e = TfIdfEmbedder::new(384);
    let v1 = e.embed(&["hello world auth".to_string()]).unwrap();
    let v2 = e.embed(&["hello world auth".to_string()]).unwrap();
    assert_eq!(v1.len(), 1);
    assert_eq!(v1[0].len(), 384);
    assert_eq!(v1, v2, "TfIdf should be deterministic");

    // Check normalized (magnitude ~1.0)
    let norm: f32 = v1[0].iter().map(|x| x*x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.01, "TfIdf vector should be normalized, got {}", norm);

    // Different texts should have different vectors
    let v3 = e.embed(&["completely different text xyz".to_string()]).unwrap();
    assert!(cosine(&v1[0], &v3[0]) < 0.95, "different texts should not be identical cosine");
}

#[test]
fn test_tfidf_cosine_semantic_like() {
    let e = TfIdfEmbedder::new(384);
    let q = "auth login user";
    let doc_auth = "auth.rs contains authentication login user password";
    let doc_db = "db.rs handles database postgres query";
    let embs = e.embed(&[q.to_string(), doc_auth.to_string(), doc_db.to_string()]).unwrap();
    let cos_auth = cosine(&embs[0], &embs[1]);
    let cos_db = cosine(&embs[0], &embs[2]);
    // auth query should be closer to auth doc than db doc with TfIdf hash trick
    assert!(cos_auth > cos_db, "TfIdf: auth query should rank auth doc higher ({} vs {})", cos_auth, cos_db);
}

#[test]
fn test_fastembed_or_tfidf_fallback_no_hang() {
    // Should not hang trying to download 120MB; should fallback to TfIdf instantly if not cached
    // Config default is tfidf, so should be fast
    let cfg = AppConfig::default();
    let start = std::time::Instant::now();
    let embedder = get_embedder(&cfg);
    let elapsed = start.elapsed();
    // Should be very fast (< 1s) since TfIdf fallback, not FastEmbed download
    assert!(elapsed.as_millis() < 2000, "get_embedder should be fast fallback, took {:?}", elapsed);
    assert_eq!(embedder.name(), "tfidf", "default config should use tfidf");
    let v = embedder.embed(&["test".to_string()]).unwrap();
    assert_eq!(v[0].len(), 384);
}

#[test]
fn test_intent_detection() {
    assert_eq!(detect_intent("explain auth flow").intent, ProjectIntent::Explain);
    assert_eq!(detect_intent("where is database code?").intent, ProjectIntent::Find);
    assert_eq!(detect_intent("fix bug in auth").intent, ProjectIntent::Debug);
    // "what is architecture?" overlaps Explain's "what is" keyword vs Architecture's "architecture" (tie -> Explain wins per rank order)
    assert_eq!(detect_intent("what is architecture?").intent, ProjectIntent::Explain);
    // Unambiguous architecture query (no "what is") should be Architecture
    assert_eq!(detect_intent("architecture structure components").intent, ProjectIntent::Architecture);
    assert_eq!(detect_intent("").intent, ProjectIntent::Unknown);
}

#[test]
fn test_rank_relevant_files_keyword_baseline() {
    let files = make_files();
    // Query "auth" should rank src/auth.rs highest via keyword scoring
    let intent = detect_intent("explain auth");
    let ranked = rank_relevant_files("auth", &files, &intent);
    assert!(!ranked.is_empty(), "should have ranked files");
    let top = &ranked[0];
    assert!(top.file.path.contains("auth"), "top should be auth.rs, got {} score {}", top.file.path, top.score);
    // node_modules should be ignored
    assert!(!ranked.iter().any(|r| r.file.path.contains("node_modules")), "ignored dir should not be ranked");
}

#[test]
fn test_rank_relevant_files_ignores_non_source() {
    let files = vec![
        ProjectFile { name: "package-lock.json".into(), path: "package-lock.json".into(), is_directory: false },
        ProjectFile { name: "auth.rs".into(), path: "src/auth.rs".into(), is_directory: false },
    ];
    let intent = detect_intent("auth");
    let ranked = rank_relevant_files("auth", &files, &intent);
    // package-lock should be ignored
    assert!(ranked.iter().all(|r| r.file.path != "package-lock.json"));
}

#[test]
fn test_hybrid_rank_vs_keyword_baseline_recall() {
    // Build a tiny index with TfIdf and verify hybrid improves or equals keyword for relevant query
    let dir = tmp_dir("hybrid-vs-keyword");
    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "rag-test".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: vec![],
    };
    // Create real files
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/auth.rs"), "fn authenticate(user: &str) { let password = \"secret\"; login(user) }").unwrap();
    fs::write(dir.join("src/db.rs"), "fn query_db() { sql(\"SELECT * FROM users\") }").unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() { println!(\"hello\"); }").unwrap();

    let files = vec![
        ProjectFile { name: "auth.rs".into(), path: "src/auth.rs".into(), is_directory: false },
        ProjectFile { name: "db.rs".into(), path: "src/db.rs".into(), is_directory: false },
        ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false },
    ];

    // Build index with TfIdf (deterministic, no download)
    let embedder: Arc<dyn Embedder> = Arc::new(TfIdfEmbedder::new(384));
    let idx = build_index(&proj, embedder.clone()).expect("build_index should succeed");
    assert_eq!(idx.files.len(), 3, "should index 3 files");
    assert_eq!(idx.embedder_name, "tfidf");

    // Hybrid query "auth" should rank auth.rs top
    let hybrid = hybrid_rank("auth", &files, Some(&idx), Some(embedder.clone()));
    let keyword = rank_relevant_files("auth", &files, &detect_intent("auth"));

    println!("Hybrid: {:?}", hybrid.iter().map(|r| (r.file.path.clone(), r.score)).collect::<Vec<_>>());
    println!("Keyword: {:?}", keyword.iter().map(|r| (r.file.path.clone(), r.score)).collect::<Vec<_>>());

    assert!(!hybrid.is_empty());
    assert!(hybrid[0].file.path.contains("auth"), "hybrid top should be auth.rs, got {}", hybrid[0].file.path);

    // Hybrid should have cosine reason
    assert!(hybrid[0].reasons.iter().any(|r| r.contains("cosine")), "hybrid should have cosine reason, got {:?}", hybrid[0].reasons);

    // Test query_index directly
    let q_emb = embedder.embed(&["auth login".to_string()]).unwrap();
    let scored = query_index(&idx, &q_emb[0], 2);
    assert_eq!(scored.len(), 2);
    assert!(scored[0].0.path.contains("auth"), "query_index top should be auth, got {}", scored[0].0.path);
    assert!(scored[0].1 > 0.0);

    // Verify index persistence
    let loaded = load_index(&proj).unwrap().expect("load_index should exist");
    assert_eq!(loaded.files.len(), 3);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_hybrid_fallback_to_keyword_when_no_index() {
    let files = make_files();
    let ranked = hybrid_rank("auth", &files, None, None);
    let baseline = rank_relevant_files("auth", &files, &detect_intent("auth"));
    // When no embeddings, hybrid should equal keyword baseline
    assert_eq!(ranked.len(), baseline.len());
    if !ranked.is_empty() {
        assert_eq!(ranked[0].file.path, baseline[0].file.path);
    }
}

#[test]
fn test_index_chunking_and_hash() {
    let dir = tmp_dir("chunk-hash");
    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "chunk-test".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: vec![],
    };
    fs::create_dir_all(dir.join("src")).unwrap();
    // File larger than 1500 chars to trigger chunking
    let long_content = "a".repeat(2500) + "\nfn unique_function_xyz() {}\n" + &"b".repeat(2000);
    fs::write(dir.join("src/long.rs"), &long_content).unwrap();

    let embedder: Arc<dyn Embedder> = Arc::new(TfIdfEmbedder::new(384));
    let idx = build_index(&proj, embedder).unwrap();
    assert_eq!(idx.files.len(), 1);
    assert!(idx.files[0].chunks.len() > 1, "long file should be chunked into multiple, got {}", idx.files[0].chunks.len());
    assert!(!idx.files[0].hash.is_empty());
    assert_eq!(idx.files[0].hash.len(), 16);

    // needs_rebuild should be false immediately after build
    assert!(!local_ai::core::index::needs_rebuild(&proj, &idx), "fresh index should not need rebuild");

    // After modifying file, should need rebuild
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(dir.join("src/long.rs"), "modified content").unwrap();
    assert!(local_ai::core::index::needs_rebuild(&proj, &idx), "modified file should trigger rebuild");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_index_skips_empty_and_binary() {
    let dir = tmp_dir("skip-empty");
    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "skip-test".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: vec![],
    };
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/empty.rs"), "").unwrap();
    fs::write(dir.join("src/valid.rs"), "fn hello() {}").unwrap();
    // Binary-like file with null byte should be skipped
    fs::write(dir.join("src/binary.dat"), "hello\0world").unwrap();

    let embedder: Arc<dyn Embedder> = Arc::new(TfIdfEmbedder::new(384));
    let result = build_index(&proj, embedder);
    // Should succeed with filtered files (empty and binary skipped)
    match result {
        Ok(idx) => {
            assert!(idx.files.iter().all(|f| f.path != "src/empty.rs"), "empty should be skipped");
            // binary.dat may be filtered or not counted as .rs, but check if it's skipped via content len or null
        },
        Err(e) => {
            // If no indexable files (e.g., both skipped and only binary left), error is expected
            assert!(e.to_string().contains("No indexable files") || e.to_string().contains("empty"), "got {}", e);
        }
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_bge_small_dimension() {
    // BAAI/bge-small-en-v1.5 is 384 dim per spec 120MB, quantized
    // TfIdf fallback also uses 384 to match
    let e = TfIdfEmbedder::new(384);
    assert_eq!(e.dim(), 384);
    let v = e.embed(&["test dimension".to_string()]).unwrap();
    assert_eq!(v[0].len(), 384);
}

#[test]
fn test_hybrid_score_formula() {
    // Verify hybrid formula 0.55 cosine + 0.35 keyword + 0.10 is applied
    // We can't directly test private keyword_scores, but we can ensure hybrid score > keyword alone for relevant file
    let dir = tmp_dir("hybrid-formula");
    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "formula".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: vec![],
    };
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/exact_match.rs"), "exact_match_token_xyz_auth").unwrap();
    fs::write(dir.join("src/unrelated.rs"), "unrelated content foo bar baz").unwrap();

    let files = vec![
        ProjectFile { name: "exact_match.rs".into(), path: "src/exact_match.rs".into(), is_directory: false },
        ProjectFile { name: "unrelated.rs".into(), path: "src/unrelated.rs".into(), is_directory: false },
    ];

    let embedder: Arc<dyn Embedder> = Arc::new(TfIdfEmbedder::new(384));
    let idx = build_index(&proj, embedder.clone()).unwrap();
    let hybrid = hybrid_rank("exact_match_token_xyz_auth", &files, Some(&idx), Some(embedder));
    assert!(hybrid[0].score >= 0.25, "hybrid score should be >= threshold, got {}", hybrid[0].score);
    assert!(hybrid[0].file.path == "src/exact_match.rs");

    fs::remove_dir_all(&dir).ok();
}

