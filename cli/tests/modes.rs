//! Plan / Build mode tests — plan can't build anything
//! Verifies: mode enum, config persistence, is_plan_mode, require_build_mode blocking

use local_ai::core::config::{AppMode, AppConfig, is_plan_mode, require_build_mode, current_mode};
use std::str::FromStr;
use std::sync::Mutex;
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_mode_enum_parse() {
    assert_eq!(AppMode::from_str("plan").unwrap(), AppMode::Plan);
    assert_eq!(AppMode::from_str("build").unwrap(), AppMode::Build);
    assert_eq!(AppMode::from_str("PLAN").unwrap(), AppMode::Plan);
    assert!(AppMode::from_str("invalid").is_err());
    assert_eq!(AppMode::Plan.to_string(), "plan");
    assert_eq!(AppMode::Build.to_string(), "build");
}

#[test]
fn test_mode_default_is_build() {
    assert_eq!(AppMode::default(), AppMode::Build);
    let cfg = AppConfig::default();
    assert_eq!(cfg.mode, AppMode::Build);
}

#[test]
fn test_is_plan_mode_via_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Save original
    let orig = std::env::var("LOCAL_AI_MODE").ok();
    // Plan via env should block
    std::env::set_var("LOCAL_AI_MODE", "plan");
    assert!(is_plan_mode(), "env plan should be plan mode");
    assert_eq!(current_mode(), AppMode::Plan);
    assert!(require_build_mode("test").is_err(), "plan should block build");
    assert!(require_build_mode("test").unwrap_err().to_string().contains("Plan mode is active"));

    // Build via env should allow
    std::env::set_var("LOCAL_AI_MODE", "build");
    assert!(!is_plan_mode(), "env build should not be plan");
    assert_eq!(current_mode(), AppMode::Build);
    assert!(require_build_mode("test").is_ok(), "build should allow");

    // Cleanup: restore original or remove
    if let Some(v) = orig {
        std::env::set_var("LOCAL_AI_MODE", v);
    } else {
        std::env::remove_var("LOCAL_AI_MODE");
    }
    // Without env, should follow config file (default build unless file says plan)
    // Default config is build, so not plan
    assert!(!is_plan_mode() || std::env::var("LOCAL_AI_MODE").map(|v| v=="plan").unwrap_or(false));
}

#[test]
fn test_mode_config_serialization() {
    let mut cfg = AppConfig::default();
    cfg.mode = AppMode::Plan;
    let toml = toml::to_string(&cfg).unwrap();
    assert!(toml.contains("plan"), "toml should contain plan, got {}", toml);
    let parsed: AppConfig = toml::from_str(&toml).unwrap();
    assert_eq!(parsed.mode, AppMode::Plan);

    cfg.mode = AppMode::Build;
    let toml2 = toml::to_string(&cfg).unwrap();
    assert!(toml2.contains("build"));
}

#[test]
fn test_browser_and_terminal_access_now_enabled() {
    // Now browser + terminal access ARE provided (as requested)
    // Verify project_tools includes exec and web_fetch for reading all necessary files
    let tools = local_ai::core::tools::project_tools();
    let names: Vec<String> = tools.iter().map(|t| t.function.name.clone()).collect();
    assert!(names.contains(&"list_project_files".to_string()));
    assert!(names.contains(&"read_project_file".to_string()));
    assert!(names.contains(&"search_project".to_string()));
    assert!(names.contains(&"exec".to_string()), "exec terminal tool should be available for reading all files via ls/cat/find/grep, got {:?}", names);
    assert!(names.contains(&"web_fetch".to_string()), "web_fetch browser tool should be available, got {:?}", names);
}

#[test]
fn test_exec_tool_reads_files_via_terminal() {
    // Use exec tool to read files via terminal (like `ls` and `cat`)
    use local_ai::core::tools::{ToolCall, execute_tool_sync};
    use local_ai::core::projects::Project;
    use std::fs;
    use serde_json::json;

    let dir = std::env::temp_dir().join(format!("local-ai-exec-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("hello.txt"), "hello terminal").unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();

    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "exec-test".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: vec![],
    };

    // Test ls via exec
    let call = ToolCall { id: None, name: "exec".into(), arguments: json!({"command": "ls -R"}).to_string(), parsed_args: json!({"command": "ls -R"}) };
    let result = execute_tool_sync(&proj, &call).unwrap();
    let stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    assert!(stdout.contains("hello.txt") || stdout.contains("main.rs"), "ls should list files, got {}", stdout);
    assert!(result.get("success").and_then(|v| v.as_bool()).unwrap_or(false));

    // Test cat via exec
    let call2 = ToolCall { id: None, name: "exec".into(), arguments: json!({"command": "cat hello.txt"}).to_string(), parsed_args: json!({"command": "cat hello.txt"}) };
    let result2 = execute_tool_sync(&proj, &call2).unwrap();
    let stdout2 = result2.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    assert!(stdout2.contains("hello terminal"), "cat should read file, got {}", stdout2);

    // Test grep via exec
    let call3 = ToolCall { id: None, name: "exec".into(), arguments: json!({"command": "grep -rn \"main\" --include=\"*.rs\""}).to_string(), parsed_args: json!({"command": "grep -rn \"main\" --include=\"*.rs\""}) };
    let result3 = execute_tool_sync(&proj, &call3).unwrap();
    let stdout3 = result3.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    assert!(stdout3.contains("main.rs") || stdout3.contains("main"), "grep should find main, got {}", stdout3);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_exec_blocked_in_plan_mode_for_write() {
    let _guard = ENV_LOCK.lock().unwrap();
    use local_ai::core::tools::{ToolCall, execute_tool_sync};
    use local_ai::core::projects::Project;
    use serde_json::json;
    use std::fs;

    let dir = std::env::temp_dir().join(format!("local-ai-plan-exec-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "plan-exec".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(dir.to_string_lossy().to_string()),
        messages: vec![],
    };

    let orig = std::env::var("LOCAL_AI_MODE").ok();
    std::env::set_var("LOCAL_AI_MODE", "plan");

    // Read-only should pass
    let call_read = ToolCall { id: None, name: "exec".into(), arguments: json!({"command": "ls -la"}).to_string(), parsed_args: json!({"command": "ls -la"}) };
    assert!(execute_tool_sync(&proj, &call_read).is_ok(), "read-only ls should pass in plan mode");

    // Write/build should be blocked
    let call_write = ToolCall { id: None, name: "exec".into(), arguments: json!({"command": "rm hello.txt"}).to_string(), parsed_args: json!({"command": "rm hello.txt"}) };
    let err = execute_tool_sync(&proj, &call_write).unwrap_err();
    assert!(err.to_string().contains("Plan mode"), "write should be blocked in plan, got {}", err);

    let call_build = ToolCall { id: None, name: "exec".into(), arguments: json!({"command": "cargo build"}).to_string(), parsed_args: json!({"command": "cargo build"}) };
    let err2 = execute_tool_sync(&proj, &call_build).unwrap_err();
    assert!(err2.to_string().contains("Plan mode"));

    if let Some(v) = orig { std::env::set_var("LOCAL_AI_MODE", v); } else { std::env::remove_var("LOCAL_AI_MODE"); }
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn test_web_fetch_browser_access() {
    let _guard = ENV_LOCK.lock().unwrap();
    use local_ai::core::tools::{ToolCall, execute_tool};
    use local_ai::core::projects::Project;
    use serde_json::json;
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Mock HTTP server for browser fetch (no external network needed)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let body = "# Mock Docs\nThis is browser fetched content for test.";
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/markdown\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        let _ = socket.write_all(resp.as_bytes()).await;
    });

    let proj = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: "web-test".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        folder_path: Some(std::env::temp_dir().to_string_lossy().to_string()),
        messages: vec![],
    };

    // Valid fetch via mock server
    let url = format!("http://{}/", addr);
    let call = ToolCall { id: None, name: "web_fetch".into(), arguments: json!({"url": url}).to_string(), parsed_args: json!({"url": url}) };
    let result = execute_tool(&proj, &call).await.unwrap();
    let content = result.get("content").and_then(|v| v.as_str()).unwrap_or("");
    assert!(content.contains("Mock Docs"), "web_fetch should return mock content, got {}", content);
    assert_eq!(result.get("status").and_then(|v| v.as_u64()).unwrap(), 200);

    // Invalid URL should be rejected
    let call_bad = ToolCall { id: None, name: "web_fetch".into(), arguments: json!({"url": "ftp://bad"}).to_string(), parsed_args: json!({"url": "ftp://bad"}) };
    let err = execute_tool(&proj, &call_bad).await.unwrap_err();
    assert!(err.to_string().contains("Invalid URL"));

    // Plan mode should still allow web_fetch (read-only browser)
    let orig = std::env::var("LOCAL_AI_MODE").ok();
    std::env::set_var("LOCAL_AI_MODE", "plan");
    let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener2.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let body = "plan mode browser still works";
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        let _ = socket.write_all(resp.as_bytes()).await;
    });
    let url2 = format!("http://{}/", addr2);
    let call2 = ToolCall { id: None, name: "web_fetch".into(), arguments: json!({"url": url2}).to_string(), parsed_args: json!({"url": url2}) };
    let result2 = execute_tool(&proj, &call2).await;
    assert!(result2.is_ok(), "web_fetch should work even in plan mode, got {:?}", result2);
    if let Some(v) = orig { std::env::set_var("LOCAL_AI_MODE", v); } else { std::env::remove_var("LOCAL_AI_MODE"); }
}
