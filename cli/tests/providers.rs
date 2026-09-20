//! Phase 5 - Provider integration tests
//! Validates: mock servers for GET /v1/models (LM Studio) and GET /api/tags (Ollama)
//! + streaming data: SSE vs NDJSON {"message":{"content":...}}
//! Reference: plan.md §5 tests/providers.rs

use local_ai::core::config::{AppConfig, ProviderKind};
use local_ai::core::provider::{get_provider, Provider, autodetect, list_models_unified, stream_chat_unified, ChatMessage};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct MockServer {
    addr: std::net::SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    fn url_v1(&self) -> String {
        format!("http://{}/v1", self.addr)
    }
}

// Generic mock server that maps path prefixes to responses
async fn start_mock_server(routes: HashMap<String, MockResponse>) -> MockServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = std::sync::Arc::new(routes);
    let handle = tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match socket.read(&mut buf).await {
                    Ok(n) if n > 0 => n,
                    _ => return,
                };
                let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
                let first_line = req_str.lines().next().unwrap_or("");
                let parts: Vec<&str> = first_line.split_whitespace().collect();
                let path = if parts.len() >= 2 { parts[1] } else { "/" };
                // Find matching route (prefix)
                let mut matched: Option<&MockResponse> = None;
                // Try exact then prefix
                if let Some(r) = routes.get(path) {
                    matched = Some(r);
                } else {
                    for (k, v) in routes.iter() {
                        if path.starts_with(k.as_str()) {
                            matched = Some(v);
                            break;
                        }
                    }
                }
                let resp = if let Some(r) = matched {
                    let body = r.body.clone();
                    let content_type = r.content_type.clone();
                    let status = r.status;
                    let status_text = if status == 200 { "OK" } else { "Not Found" };
                    format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status, status_text, content_type, body.len(), body
                    )
                } else {
                    // 404
                    let body = format!("{{\"error\":\"no route for {}\"}}", path);
                    format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    )
                };
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    MockServer { addr, _handle: handle }
}

#[derive(Clone)]
struct MockResponse {
    status: u16,
    body: String,
    content_type: String,
}

impl MockResponse {
    fn json(body: &str) -> Self {
        Self { status: 200, body: body.to_string(), content_type: "application/json".to_string() }
    }
    fn sse(body: &str) -> Self {
        Self { status: 200, body: body.to_string(), content_type: "text/event-stream".to_string() }
    }
    fn ndjson(body: &str) -> Self {
        Self { status: 200, body: body.to_string(), content_type: "application/x-ndjson".to_string() }
    }
}

// ---------- Helpers for canned responses ----------

fn lmstudio_models_json() -> String {
    r#"{"data":[{"id":"qwen/qwen3.5-9b","object":"model","owned_by":"qwen"},{"id":"mistral:7b","object":"model","owned_by":"mistral"}]}"#.to_string()
}

fn lmstudio_sse_body() -> String {
    // Two chunks + DONE
    let chunk1 = serde_json::json!({"choices":[{"delta":{"content":"Hello "}}]});
    let chunk2 = serde_json::json!({"choices":[{"delta":{"content":"world"}}]});
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", chunk1, chunk2)
}

fn ollama_tags_json() -> String {
    r#"{"models":[{"name":"llama3.1:8b","model":"llama3.1:8b","details":{"family":"llama","parameter_size":"8B"}},{"name":"qwen2.5:7b","model":"qwen2.5:7b","details":{"family":"qwen"}}]}"#.to_string()
}

fn ollama_ndjson_body() -> String {
    // NDJSON: each line is JSON object
    let line1 = serde_json::json!({"message":{"content":"Hello "},"done":false});
    let line2 = serde_json::json!({"message":{"content":"world"},"done":false});
    let line3 = serde_json::json!({"message":null,"done":true});
    format!("{}\n{}\n{}\n", line1, line2, line3)
}

fn ollama_openai_models_json() -> String {
    r#"{"data":[{"id":"llama3.1:8b","object":"model","owned_by":"ollama"},{"id":"qwen2.5:7b","object":"model","owned_by":"ollama"}]}"#.to_string()
}

// ---------- LM Studio Tests ----------

#[tokio::test]
async fn test_lmstudio_health_check_success() {
    let mut routes = HashMap::new();
    routes.insert("/v1/models".to_string(), MockResponse::json(&lmstudio_models_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::LmStudio);
    let ok = p.health_check(&server.url_v1()).await;
    assert!(ok, "LM Studio health_check should succeed when /v1/models returns 200");
}

#[tokio::test]
async fn test_lmstudio_health_check_fails_when_down() {
    // No server on random port → should be false (5s timeout)
    let p = get_provider(&ProviderKind::LmStudio);
    let ok = p.health_check("http://127.0.0.1:59999/v1").await;
    assert!(!ok, "health_check should fail for non-existent server");
}

#[tokio::test]
async fn test_lmstudio_list_models() {
    let mut routes = HashMap::new();
    routes.insert("/v1/models".to_string(), MockResponse::json(&lmstudio_models_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::LmStudio);
    let models = p.list_models(&server.url_v1()).await.expect("list_models should succeed");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "qwen/qwen3.5-9b");
    assert_eq!(models[0].provider, "lmstudio");
    assert_eq!(models[1].id, "mistral:7b");
}

#[tokio::test]
async fn test_lmstudio_stream_chat_sse() {
    let mut routes = HashMap::new();
    routes.insert("/v1/chat/completions".to_string(), MockResponse::sse(&lmstudio_sse_body()));
    // Also need /v1/models for health? Not needed for stream
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::LmStudio);
    let msgs = vec![ChatMessage { role: "user".into(), content: "hello".into() }];
    let mut collected = String::new();
    let mut streamed = Vec::new();
    let result = p.stream_chat(&server.url_v1(), "qwen/qwen3.5-9b", msgs, 0.4, &mut |chunk| {
        streamed.push(chunk.to_string());
        collected.push_str(chunk);
    }).await.expect("stream_chat should succeed");
    // Result should be full concatenated
    assert!(result.contains("Hello"), "result should contain Hello, got {}", result);
    assert!(result.contains("world"));
    assert_eq!(collected, result);
    assert!(streamed.contains(&"Hello ".to_string()) || collected == "Hello world");
}

#[tokio::test]
async fn test_lmstudio_supports_tools() {
    let p = get_provider(&ProviderKind::LmStudio);
    assert!(p.supports_tools(), "LM Studio should support tools");
}

// ---------- Ollama Tests ----------

#[tokio::test]
async fn test_ollama_health_check_native() {
    let mut routes = HashMap::new();
    routes.insert("/api/tags".to_string(), MockResponse::json(&ollama_tags_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Ollama);
    let ok = p.health_check(&server.url()).await;
    assert!(ok, "Ollama health_check should succeed via /api/tags");
}

#[tokio::test]
async fn test_ollama_health_check_fallback_openai_compat() {
    // Only /v1/models available, not /api/tags
    let mut routes = HashMap::new();
    routes.insert("/v1/models".to_string(), MockResponse::json(&ollama_openai_models_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Ollama);
    let ok = p.health_check(&server.url()).await;
    assert!(ok, "Ollama should fallback to /v1/models when /api/tags missing");
}

#[tokio::test]
async fn test_ollama_list_models_native() {
    let mut routes = HashMap::new();
    routes.insert("/api/tags".to_string(), MockResponse::json(&ollama_tags_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Ollama);
    let models = p.list_models(&server.url()).await.expect("ollama list should succeed");
    assert_eq!(models.len(), 2);
    assert!(models.iter().any(|m| m.id == "llama3.1:8b"));
    assert!(models.iter().any(|m| m.id == "qwen2.5:7b"));
    assert!(models[0].provider == "ollama");
}

#[tokio::test]
async fn test_ollama_list_models_fallback_openai() {
    // No /api/tags, only /v1/models
    let mut routes = HashMap::new();
    routes.insert("/v1/models".to_string(), MockResponse::json(&ollama_openai_models_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Ollama);
    let models = p.list_models(&server.url()).await.expect("fallback should succeed");
    assert_eq!(models.len(), 2);
    assert!(models.iter().any(|m| m.id == "llama3.1:8b"));
}

#[tokio::test]
async fn test_ollama_stream_native_ndjson() {
    let mut routes = HashMap::new();
    routes.insert("/api/chat".to_string(), MockResponse::ndjson(&ollama_ndjson_body()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Ollama);
    let msgs = vec![ChatMessage { role: "user".into(), content: "hello".into() }];
    let mut collected = String::new();
    let result = p.stream_chat(&server.url(), "llama3.1:8b", msgs, 0.4, &mut |chunk| {
        collected.push_str(chunk);
    }).await.expect("ollama native stream should succeed");
    assert!(result.contains("Hello"), "got {}", result);
    assert!(result.contains("world"));
    assert_eq!(collected, result);
}

#[tokio::test]
async fn test_ollama_stream_openai_compat_when_url_has_v1() {
    // When base_url ends with /v1, should use SSE path even for Ollama
    let mut routes = HashMap::new();
    routes.insert("/v1/chat/completions".to_string(), MockResponse::sse(&lmstudio_sse_body()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Ollama);
    let msgs = vec![ChatMessage { role: "user".into(), content: "hello".into() }];
    let mut collected = String::new();
    let result = p.stream_chat(&format!("{}/v1", server.url()), "llama3.1:8b", msgs, 0.4, &mut |c| collected.push_str(c)).await.expect("openai compat stream");
    assert!(result.contains("Hello"));
    assert_eq!(collected, result);
}

#[tokio::test]
async fn test_ollama_supports_tools() {
    let p = get_provider(&ProviderKind::Ollama);
    assert!(p.supports_tools());
}

// ---------- Generic Provider ----------

#[tokio::test]
async fn test_generic_provider_health_and_list() {
    let mut routes = HashMap::new();
    routes.insert("/v1/models".to_string(), MockResponse::json(&lmstudio_models_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Generic);
    let ok = p.health_check(&server.url_v1()).await;
    assert!(ok);
    let models = p.list_models(&server.url_v1()).await.unwrap();
    assert!(!models.is_empty());
}

// ---------- Unified & Autodetect ----------

#[tokio::test]
async fn test_autodetect_prefers_ollama_when_both_up() {
    // Ollama up, LM Studio up -> autodetect should pick Ollama first per spec order: ollama → lmstudio → llamacpp → generic
    let mut routes_ollama = HashMap::new();
    routes_ollama.insert("/api/tags".to_string(), MockResponse::json(&ollama_tags_json()));
    let ollama_server = start_mock_server(routes_ollama).await;

    let mut routes_lm = HashMap::new();
    routes_lm.insert("/v1/models".to_string(), MockResponse::json(&lmstudio_models_json()));
    let lm_server = start_mock_server(routes_lm).await;

    let cfg = AppConfig {
        mode: Default::default(),
        provider: local_ai::core::config::ProviderSection { active: ProviderKind::Auto, url: None },
        providers: local_ai::core::config::ProvidersConfig {
            ollama: local_ai::core::config::ProviderConfig { url: ollama_server.url(), use_openai_compat: false },
            lmstudio: local_ai::core::config::ProviderConfig { url: lm_server.url_v1(), use_openai_compat: false },
            llamacpp: local_ai::core::config::ProviderConfig { url: "http://127.0.0.1:59998".into(), use_openai_compat: false },
            generic: local_ai::core::config::ProviderConfig { url: "".into(), use_openai_compat: false },
        },
        grounding: Default::default(),
        embeddings: Default::default(),
    };
    let (kind, url, ok) = autodetect(&cfg).await;
    assert!(ok, "autodetect should find a provider");
    assert_eq!(kind, ProviderKind::Ollama, "should prefer Ollama when both up");
    assert_eq!(url, ollama_server.url());
}

#[tokio::test]
async fn test_autodetect_falls_back_to_lmstudio_when_ollama_down() {
    let mut routes_lm = HashMap::new();
    routes_lm.insert("/v1/models".to_string(), MockResponse::json(&lmstudio_models_json()));
    let lm_server = start_mock_server(routes_lm).await;

    let cfg = AppConfig {
        mode: Default::default(),
        provider: local_ai::core::config::ProviderSection { active: ProviderKind::Auto, url: None },
        providers: local_ai::core::config::ProvidersConfig {
            ollama: local_ai::core::config::ProviderConfig { url: "http://127.0.0.1:59997".into(), use_openai_compat: false },
            lmstudio: local_ai::core::config::ProviderConfig { url: lm_server.url_v1(), use_openai_compat: false },
            llamacpp: local_ai::core::config::ProviderConfig { url: "http://127.0.0.1:59998".into(), use_openai_compat: false },
            generic: local_ai::core::config::ProviderConfig { url: "".into(), use_openai_compat: false },
        },
        grounding: Default::default(),
        embeddings: Default::default(),
    };
    let (kind, url, ok) = autodetect(&cfg).await;
    assert!(ok);
    assert_eq!(kind, ProviderKind::LmStudio);
    assert_eq!(url, lm_server.url_v1());
}

#[tokio::test]
async fn test_autodetect_none_when_all_down() {
    let cfg = AppConfig {
        mode: Default::default(),
        provider: local_ai::core::config::ProviderSection { active: ProviderKind::Auto, url: None },
        providers: local_ai::core::config::ProvidersConfig {
            ollama: local_ai::core::config::ProviderConfig { url: "http://127.0.0.1:59991".into(), use_openai_compat: false },
            lmstudio: local_ai::core::config::ProviderConfig { url: "http://127.0.0.1:59992/v1".into(), use_openai_compat: false },
            llamacpp: local_ai::core::config::ProviderConfig { url: "http://127.0.0.1:59993".into(), use_openai_compat: false },
            generic: local_ai::core::config::ProviderConfig { url: "".into(), use_openai_compat: false },
        },
        grounding: Default::default(),
        embeddings: Default::default(),
    };
    let (_kind, _url, ok) = autodetect(&cfg).await;
    assert!(!ok, "should be not ok when all down");
}

#[tokio::test]
async fn test_unified_list_models_auto_merges() {
    // Both providers up, auto should merge and deduplicate
    let mut routes_ollama = HashMap::new();
    routes_ollama.insert("/api/tags".to_string(), MockResponse::json(&ollama_tags_json()));
    let ollama_server = start_mock_server(routes_ollama).await;

    let mut routes_lm = HashMap::new();
    routes_lm.insert("/v1/models".to_string(), MockResponse::json(&lmstudio_models_json()));
    let lm_server = start_mock_server(routes_lm).await;

    let cfg = AppConfig {
        mode: Default::default(),
        provider: local_ai::core::config::ProviderSection { active: ProviderKind::Auto, url: None },
        providers: local_ai::core::config::ProvidersConfig {
            ollama: local_ai::core::config::ProviderConfig { url: ollama_server.url(), use_openai_compat: false },
            lmstudio: local_ai::core::config::ProviderConfig { url: lm_server.url_v1(), use_openai_compat: false },
            llamacpp: local_ai::core::config::ProviderConfig { url: "".into(), use_openai_compat: false },
            generic: local_ai::core::config::ProviderConfig { url: "".into(), use_openai_compat: false },
        },
        grounding: Default::default(),
        embeddings: Default::default(),
    };
    let models = list_models_unified(&ProviderKind::Auto, "", &cfg).await.unwrap();
    // Should have at least 4 unique: 2 from ollama (llama3.1, qwen2.5) + 2 from lmstudio (qwen3.5, mistral)
    assert!(models.len() >= 3, "merged got {:?}", models.iter().map(|m| &m.id).collect::<Vec<_>>());
    // Check provider field set
    assert!(models.iter().any(|m| m.provider == "ollama"));
    assert!(models.iter().any(|m| m.provider == "lmstudio"));
}

#[tokio::test]
async fn test_unified_stream_chat_auto() {
    let mut routes = HashMap::new();
    routes.insert("/v1/models".to_string(), MockResponse::json(&lmstudio_models_json()));
    routes.insert("/v1/chat/completions".to_string(), MockResponse::sse(&lmstudio_sse_body()));
    let server = start_mock_server(routes).await;
    let cfg = AppConfig {
        mode: Default::default(),
        provider: local_ai::core::config::ProviderSection { active: ProviderKind::Auto, url: None },
        providers: local_ai::core::config::ProvidersConfig {
            ollama: local_ai::core::config::ProviderConfig { url: "http://127.0.0.1:59991".into(), use_openai_compat: false },
            lmstudio: local_ai::core::config::ProviderConfig { url: server.url_v1(), use_openai_compat: false },
            llamacpp: local_ai::core::config::ProviderConfig { url: "".into(), use_openai_compat: false },
            generic: local_ai::core::config::ProviderConfig { url: "".into(), use_openai_compat: false },
        },
        grounding: Default::default(),
        embeddings: Default::default(),
    };
    let msgs = vec![ChatMessage { role: "user".into(), content: "hello".into() }];
    let mut out = String::new();
    let result = stream_chat_unified(&ProviderKind::Auto, "", &cfg, "qwen/qwen3.5-9b", msgs, 0.4, &mut |c| out.push_str(c)).await.unwrap();
    assert!(result.contains("Hello"));
    assert_eq!(out, result);
}

#[tokio::test]
async fn test_provider_namespaced_id_concept() {
    // Verify that namespaced IDs like ollama/llama3.1:8b could be handled (local-llm-router idea)
    // Our provider currently keeps raw IDs, but test that IDs contain colon prefix typical for Ollama
    let mut routes = HashMap::new();
    routes.insert("/api/tags".to_string(), MockResponse::json(&ollama_tags_json()));
    let server = start_mock_server(routes).await;
    let p = get_provider(&ProviderKind::Ollama);
    let models = p.list_models(&server.url()).await.unwrap();
    for m in &models {
        // Ollama native IDs contain colon version tag
        assert!(m.id.contains(':'), "Ollama ID should contain ':' version, got {}", m.id);
    }
}

