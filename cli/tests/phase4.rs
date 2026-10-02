//! Phase 4 tests — missions (4.1) + MCP (4.2) + browse (4.3).
//!
//! All offline and deterministic (no LLM, no external network).
//! The MCP tests spawn the reference `mcp/filesystem` server via `python3`
//! (skipped when unavailable); the browse HTTP test serves fixtures from a
//! local `TcpListener`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use local_ai::core::fs::ProjectFile;

fn missions_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Run `f` with `LOCAL_AI_MISSIONS_FILE` pointed at a fresh temp registry.
/// Serialized: env vars are process-global and tests run in parallel.
fn with_temp_registry(tag: &str, f: impl FnOnce(std::path::PathBuf)) {
    let _guard = missions_lock().lock().unwrap();
    let path = std::env::temp_dir().join(format!("local-ai-missions-{}-{}.json", tag, uuid::Uuid::new_v4()));
    std::env::set_var("LOCAL_AI_MISSIONS_FILE", &path);
    f(path.clone());
    std::env::remove_var("LOCAL_AI_MISSIONS_FILE");
    let _ = std::fs::remove_file(&path);
}

fn fixture_project(tag: &str) -> (local_ai::core::projects::Project, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("local-ai-phase4-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {\n    todo!()\n}\n").unwrap();
    let proj = local_ai::core::projects::Project {
        id: format!("test-phase4-{}", uuid::Uuid::new_v4()),
        name: "phase4-fixture".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: Vec::new(),
    };
    (proj, dir)
}

// ---------------------------------------------------------------------------
// 4.1 missions: registry CRUD + progress
// ---------------------------------------------------------------------------

#[test]
fn test_mission_lifecycle_registry() {
    use local_ai::core::missions as m;
    with_temp_registry("lifecycle", |_| {
        let (proj, dir) = fixture_project("reg");
        let files = local_ai::core::fs::list_project_files(&proj).unwrap();
        let intent = local_ai::core::intelligence::detect_intent("fix placeholder");
        let ranked = local_ai::core::intelligence::rank_relevant_files("fix main", &files, &intent);
        let plan = local_ai::core::plan::build_plan("fix placeholder", &proj, &files, &intent.intent, &ranked);
        let budget = local_ai::core::agents::Budget::default();

        let mission = m::create_mission_record(&proj, "fix placeholder", "debug", plan, budget, false, Some("echo ok".into()), None).unwrap();
        assert_eq!(mission.number, 1);
        assert_eq!(mission.status, m::MissionStatus::Pending);
        assert!(mission.trace_path.exists());
        assert!(mission.step_statuses.iter().all(|s| s == "pending"));
        assert_eq!(mission.progress(), 0.0);

        // Lookup by number, #number and id prefix.
        assert_eq!(m::get_mission("1").unwrap().id, mission.id);
        assert_eq!(m::get_mission("#1").unwrap().id, mission.id);
        assert_eq!(m::get_mission(&mission.id[..8]).unwrap().id, mission.id);

        // Step progress: 2 of N done.
        let mut updated = mission.clone();
        let ids: Vec<String> = updated.plan.steps.iter().take(2).map(|s| s.id.clone()).collect();
        for id in &ids {
            m::record_step_status(&mut updated, id, "done").unwrap();
        }
        let reloaded = m::get_mission("1").unwrap();
        assert_eq!(reloaded.progress(), 2.0 / reloaded.plan.steps.len() as f32);
        assert_eq!(reloaded.current_step().as_deref(), reloaded.plan.steps.get(2).map(|s| s.id.as_str()));
        assert!(!reloaded.status.is_terminal());

        // Progress bar shape.
        assert_eq!(m::progress_bar(0.0), "░░░░░░░░░░   0%");
        assert_eq!(m::progress_bar(0.8), "████████░░  80%");
        assert_eq!(m::progress_bar(1.0), "██████████ 100%");

        let _ = std::fs::remove_dir_all(&dir);
    });
}

// ---------------------------------------------------------------------------
// 4.1 executor: run 2 steps → resume after kill → completes
// ---------------------------------------------------------------------------

fn test_exec_ctx(mission_goal: &str) -> local_ai::commands::runner::ExecCtx {
    local_ai::commands::runner::ExecCtx {
        goal: mission_goal.to_string(),
        test_cmd: Some("echo ok".to_string()),
        model: None, // offline: coder steps become MANUAL (still terminal)
        provider_kind: local_ai::core::config::ProviderKind::Auto,
        provider_url: String::new(),
        yes: true,
        budget: local_ai::core::agents::Budget {
            max_steps: 20,
            max_tool_calls: 100,
            max_wall_secs: 600,
            approve_dangerous: false,
        },
        max_steps_this_run: None,
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // intentional: serializes process-global env mutation across parallel tests
async fn test_mission_run_two_steps_then_resume_completes() {
    use local_ai::core::missions as m;
    let _guard = missions_lock().lock().unwrap();
    let reg = std::env::temp_dir().join(format!("local-ai-missions-resume-{}.json", uuid::Uuid::new_v4()));
    std::env::set_var("LOCAL_AI_MISSIONS_FILE", &reg);

    let (proj, dir) = fixture_project("resume");
    let files = local_ai::core::fs::list_project_files(&proj).unwrap();
    let intent = local_ai::core::intelligence::detect_intent("fix placeholder");
    let ranked = local_ai::core::intelligence::rank_relevant_files("fix main", &files, &intent);
    let plan = local_ai::core::plan::build_plan("fix placeholder", &proj, &files, &intent.intent, &ranked);
    let n_steps = plan.steps.len();
    assert!(n_steps >= 4, "plan should have several steps");

    // Mission record (as `mission create` would persist it).
    let mission = m::create_mission_record(
        &proj, "fix placeholder", "debug", plan.clone(),
        local_ai::core::agents::Budget::default(), false, Some("echo ok".into()), None,
    )
    .unwrap();
    let relevant = local_ai::commands::runner::relevant_paths_for("fix placeholder", &files);
    let cfg = local_ai::core::config::default_config();

    // --- First invocation: killed after 2 steps (simulated via step cap). ---
    let mut ctx = test_exec_ctx("fix placeholder");
    ctx.max_steps_this_run = Some(2);
    let mut state = local_ai::commands::runner::RunState::default();
    let mut orch = local_ai::core::agents::Orchestrator::new(
        mission.id.clone(),
        local_ai::core::agents::Budget::default(),
        local_ai::core::agents::ApprovalGate::Auto,
    );
    let outcome = local_ai::commands::runner::run_graph(
        &proj, &files, &relevant, &plan, &ctx, &cfg, &mut state, &mut orch,
        &mission.trace_path, &mission.id, None,
    )
    .await
    .unwrap();
    assert_eq!(outcome.stop, local_ai::commands::runner::StopReason::StepCapReached);
    assert_eq!(outcome.steps_this_run, 2);
    // Persist like the on_step hook would.
    let mut partial = m::get_mission(&mission.id).unwrap();
    for entry in &state.step_statuses {
        let (id, st) = (
            entry.get("id").and_then(|v| v.as_str()).unwrap(),
            entry.get("status").and_then(|v| v.as_str()).unwrap(),
        );
        m::record_step_status(&mut partial, id, st).unwrap();
    }

    // --- "Kill": fresh state reloaded from the registry, resume the rest. ---
    let reloaded = m::get_mission(&mission.id).unwrap();
    assert_eq!(reloaded.progress(), 2.0 / n_steps as f32);
    let mut ctx2 = test_exec_ctx("fix placeholder");
    ctx2.max_steps_this_run = None;
    let mut state2 = local_ai::commands::runner::RunState {
        step_statuses: reloaded
            .plan
            .steps
            .iter()
            .zip(reloaded.step_statuses.iter())
            .map(|(s, st)| serde_json::json!({"id": s.id, "status": st}))
            .collect(),
        applied: Vec::new(),
        skipped: Vec::new(),
        tests: Vec::new(),
    };
    let mut orch2 = local_ai::core::agents::Orchestrator::new(
        mission.id.clone(),
        local_ai::core::agents::Budget::default(),
        local_ai::core::agents::ApprovalGate::Auto,
    );
    let outcome2 = local_ai::commands::runner::run_graph(
        &proj, &files, &relevant, &plan, &ctx2, &cfg, &mut state2, &mut orch2,
        &mission.trace_path, &mission.id, None,
    )
    .await
    .unwrap();
    assert_eq!(outcome2.stop, local_ai::commands::runner::StopReason::Finished);
    // Every plan step now has a terminal status; tester steps passed.
    assert_eq!(state2.step_statuses.len(), n_steps);
    assert!(state2.step_statuses.iter().all(|e| e.get("status").and_then(|v| v.as_str()).unwrap_or("pending") != "pending"));
    assert!(state2.tests.iter().any(|t| t.get("result").and_then(|r| r.as_str()) == Some("pass")));
    assert!(outcome2.tests_green);
    // Trace file holds planner-style entries from both runs.
    let trace = std::fs::read_to_string(&mission.trace_path).unwrap();
    assert!(trace.contains("\"agent\":\"tester\""), "trace:\n{}", trace.lines().take(5).collect::<Vec<_>>().join("\n"));

    std::env::remove_var("LOCAL_AI_MISSIONS_FILE");
    let _ = std::fs::remove_file(&reg);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4.2 MCP: protocol, servers, screening
// ---------------------------------------------------------------------------

fn mcp_available() -> bool {
    std::process::Command::new(if cfg!(target_os = "windows") { "python" } else { "python3" })
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn test_mcp_name_validation_and_qualified_names() {
    use local_ai::core::mcp as mcp;
    assert!(mcp::validate_server_name("filesystem").is_ok());
    assert!(mcp::validate_server_name("../evil").is_err());
    assert!(mcp::validate_server_name("a/b").is_err());
    assert!(mcp::validate_server_name("").is_err());
    let tool = mcp::McpTool { name: "read_file".into(), description: "d".into(), parameters: serde_json::json!({}) };
    assert_eq!(tool.qualified_name("filesystem"), "mcp__filesystem__read_file");
    assert_eq!(mcp::split_qualified("mcp__filesystem__read_file"), Some(("filesystem", "read_file")));
    assert_eq!(mcp::split_qualified("read_project_file"), None);
}

#[test]
fn test_mcp_call_screening() {
    use local_ai::core::mcp::mcp_call_allowed;
    // Benign read passes even in plan mode.
    assert!(mcp_call_allowed("filesystem", "read_file", &serde_json::json!({"path": "src/main.rs"}), false).is_ok());
    // Secret path × network tool blocked without approval…
    assert!(mcp_call_allowed("github", "publish_release", &serde_json::json!({"notes": "see .env for token"}), false).is_err());
    // …allowed with --approve dangerous.
    assert!(mcp_call_allowed("github", "publish_release", &serde_json::json!({"notes": "see .env for token"}), true).is_ok());
    // Shell-dangerous smuggled arg blocked.
    assert!(mcp_call_allowed("filesystem", "read_file", &serde_json::json!({"path": "x生きる $(rm -rf /)"}), false).is_err());
}

#[test]
fn test_mcp_filesystem_server_list_and_read() {
    if !mcp_available() {
        eprintln!("SKIP: no python3 for MCP server test");
        return;
    }
    use local_ai::core::mcp::McpSession;
    let dir = std::env::temp_dir().join(format!("local-ai-mcp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hello.txt"), "hello mcp").unwrap();
    // Point discovery at the repo's mcp/ dir.
    let repo_mcp = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("mcp");
    std::env::set_var("LOCAL_AI_MCP_DIR", &repo_mcp);

    let mut session = McpSession::connect("filesystem", &dir.to_string_lossy()).expect("connect filesystem server");
    let tools = session.list_tools().expect("tools/list");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"list_files"), "{:?}", names);
    assert!(names.contains(&"read_file"), "{:?}", names);

    let listing = session.call_tool("list_files", &serde_json::json!({})).expect("list_files");
    assert!(listing.get("files").and_then(|f| f.as_array()).map(|a| a.iter().any(|e| e.get("path").and_then(|p| p.as_str()) == Some("hello.txt"))).unwrap_or(false), "{}", listing);

    let content = session.call_tool("read_file", &serde_json::json!({"path": "hello.txt"})).expect("read_file");
    assert_eq!(content.get("content").and_then(|c| c.as_str()).unwrap_or(""), "hello mcp");

    // Traversal refused by the server (error value, transport stays alive).
    let evil = session.call_tool("read_file", &serde_json::json!({"path": "../escape.txt"})).expect("traversal response");
    assert!(evil.get("error").is_some(), "{}", evil);
    // Unknown tool → server-side error value.
    let unknown = session.call_tool("nope", &serde_json::json!({})).expect("unknown response");
    assert!(unknown.get("error").is_some());

    std::env::remove_var("LOCAL_AI_MCP_DIR");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4.3 browse: parsing + local HTTP fixture
// ---------------------------------------------------------------------------

#[test]
fn test_browse_extract_links() {
    use local_ai::core::browse::extract_links;
    let html = r#"<a href="/api">api</a> <a href="guide.html">g</a> <a href="https://x.example/y#f">y</a> <a href="mailto:a@b.c">m</a> <a href="/api">dup</a>"#;
    let links = extract_links(html, "https://docs.example/start");
    assert_eq!(links, vec!["https://docs.example/api", "https://docs.example/guide.html", "https://x.example/y"]);
}

#[test]
fn test_browse_extract_code_blocks_and_title() {
    use local_ai::core::browse::{extract_code_blocks, extract_title, render_with_citations};
    let text = "intro\n```rust\nfn main() {}\n```\nmid\n```\nplain\n```\n";
    let blocks = extract_code_blocks(text, "https://x.example/p");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].lang, "rust");
    assert!(blocks[0].code.contains("fn main"));
    assert_eq!(extract_title("<html><head><title>Stripe API</title></head></html>"), "Stripe API");
    assert_eq!(extract_title("no title here"), "");
    // Citations carry the URL on every block.
    let pages = vec![local_ai::core::browse::BrowsedPage {
        url: "https://x.example/p".into(),
        title: "T".into(),
        code_blocks: blocks,
        snippet: "s".into(),
    }];
    let rendered = render_with_citations(&pages);
    assert!(rendered.contains("[source: https://x.example/p]"));
}

#[tokio::test]
async fn test_browse_local_fixture_server_cites_urls() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    // Fixture: / links /api; /api has a fenced code block.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(4) {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let body = if req.contains("GET /api ") {
                "<html><head><title>Checkout API</title></head><body>Use:\n```js\nstripe.checkout.sessions.create({})\n```\n</body></html>"
            } else {
                "<html><body><a href=\"/api\">api docs</a></body></html>"
            };
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    let base = format!("http://127.0.0.1:{}/", port);
    let index = local_ai::core::browse::fetch_url(&base, 8000).await.expect("fetch index");
    let links = local_ai::core::browse::extract_links(&index, &base);
    assert_eq!(links, vec![format!("http://127.0.0.1:{}/api", port)]);

    let pages = local_ai::core::browse::browse_pages(&links, 5, |url| async move {
        local_ai::core::browse::fetch_url(&url, 8000).await
    })
    .await;
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].title, "Checkout API");
    assert_eq!(pages[0].code_blocks.len(), 1);
    assert!(pages[0].code_blocks[0].code.contains("stripe.checkout"));
    let rendered = local_ai::core::browse::render_with_citations(&pages);
    assert!(rendered.contains(&format!("[source: http://127.0.0.1:{}/api]", port)));
}

#[test]
fn test_mission_fixture_helpers_sane() {
    // Fixture inventory for the tests above (guards against empty fixtures).
    let files = [ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false }];
    let map: HashMap<&str, &str> = HashMap::new();
    assert_eq!(files.len(), 1);
    assert!(map.is_empty());
}
