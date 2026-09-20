use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::{AIModel, ChatMessage, Provider};
use crate::core::tools::{ToolCall, ToolDefinition};

pub struct OllamaProvider;

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Option<Vec<TagModel>>,
}
#[derive(Debug, Deserialize)]
struct TagModel {
    name: String,
    model: Option<String>,
    modified_at: Option<String>,
    size: Option<u64>,
    digest: Option<String>,
    details: Option<TagDetails>,
}
#[derive(Debug, Deserialize)]
struct TagDetails {
    format: Option<String>,
    family: Option<String>,
    parameter_size: Option<String>,
}

// Ollama OpenAI compat models response
#[derive(Debug, Deserialize)]
struct OpenAIModels { data: Vec<OpenAIModel> }
#[derive(Debug, Deserialize)]
struct OpenAIModel { id: String, object: String, owned_by: String }

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaChatChunk {
    message: Option<OllamaMessage>,
    done: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct OllamaMessage { content: Option<String> }

#[async_trait::async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str { "ollama" }
    fn default_url(&self) -> &str { "http://localhost:11434" }
    fn supports_tools(&self) -> bool { true }

    async fn health_check(&self, base_url: &str) -> bool {
        let base = base_url.trim_end_matches('/');
        // Try native /api/tags first, then /v1/models
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build();
        let Ok(c) = client else { return false };
        // native
        if c.get(format!("{}/api/tags", base)).send().await.map(|r| r.status().is_success()).unwrap_or(false) {
            return true;
        }
        // openai compat
        let openai_base = if base.ends_with("/v1") { base.to_string() } else { format!("{}/v1", base) };
        c.get(format!("{}/models", openai_base)).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    async fn list_models(&self, base_url: &str) -> Result<Vec<AIModel>> {
        let base = base_url.trim_end_matches('/');
        let client = reqwest::Client::new();
        // Try native first
        let tags_url = format!("{}/api/tags", base);
        let resp = client.get(&tags_url).send().await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                let json: TagsResponse = r.json().await.context("Failed to parse Ollama /api/tags")?;
                if let Some(models) = json.models {
                    return Ok(models.into_iter().map(|m| {
                        let id = m.name.clone();
                        let owned_by = m.details.and_then(|d| d.family).unwrap_or_else(|| "ollama".to_string());
                        AIModel { id, object: "model".into(), owned_by, provider: "ollama".into() }
                    }).collect());
                }
            }
        }
        // Fallback to OpenAI compat
        let openai_base = if base.ends_with("/v1") { base.to_string() } else { format!("{}/v1", base) };
        let url = format!("{}/models", openai_base);
        let resp = client.get(&url).send().await.context("Failed to connect to Ollama (both /api/tags and /v1/models failed)")?;
        if !resp.status().is_success() {
            anyhow::bail!("Ollama returned HTTP {}", resp.status());
        }
        let json: OpenAIModels = resp.json().await.context("Failed to parse Ollama OpenAI models")?;
        Ok(json.data.into_iter().map(|m| AIModel { id: m.id, object: m.object, owned_by: m.owned_by, provider: "ollama".into() }).collect())
    }

    async fn stream_chat(
        &self,
        base_url: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        temperature: f32,
        on_chunk: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<String>
    {
        let base = base_url.trim_end_matches('/');
        // Prefer native /api/chat which is guaranteed for Ollama, fallback to /v1/chat/completions if base already is openai compat
        let use_openai = base.ends_with("/v1");
        if use_openai {
            return self.stream_openai_compat(base, model, messages, on_chunk).await;
        }
        // Try native
        match self.stream_native(base, model, messages.clone(), on_chunk).await {
            Ok(s) => Ok(s),
            Err(e) => {
                // Fallback to openai compat if native fails (e.g., old ollama not supporting /api/chat streaming the same way)
                eprintln!("Ollama native chat failed ({}), trying OpenAI compat...", e);
                let openai_base = format!("{}/v1", base);
                self.stream_openai_compat(&openai_base, model, messages, on_chunk).await
            }
        }
    }

    async fn chat_with_tools(
        &self,
        base_url: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        temperature: f32,
    ) -> Result<(String, Vec<ToolCall>)> {
        let base = base_url.trim_end_matches('/');
        // Try native Ollama /api/chat first
        let tools_json = serde_json::to_value(&tools).unwrap_or(serde_json::json!([]));
        // Ollama native expects tools as array of {type, function: {name, description, parameters}}
        let client = reqwest::Client::new();
        // Try native
        let body_native = serde_json::json!({
            "model": model,
            "messages": messages,
            "tools": tools_json,
            "stream": false,
            "temperature": temperature
        });
        let resp = client.post(format!("{}/api/chat", base)).json(&body_native).send().await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                let json: serde_json::Value = r.json().await.unwrap_or(serde_json::json!({}));
                let content = json.pointer("/message/content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let mut calls = Vec::new();
                if let Some(arr) = json.pointer("/message/tool_calls").and_then(|v| v.as_array()) {
                    for item in arr {
                        let name = item.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if name.is_empty() { continue; }
                        let args_val = item.pointer("/function/arguments");
                        let (args_str, parsed) = match args_val {
                            Some(v) if v.is_object() => (v.to_string(), v.clone()),
                            Some(v) if v.is_string() => {
                                let s = v.as_str().unwrap_or("{}").to_string();
                                let p: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));
                                (s, p)
                            }
                            _ => ("{}".to_string(), serde_json::json!({})),
                        };
                        calls.push(ToolCall { id: None, name, arguments: args_str, parsed_args: parsed });
                    }
                }
                if !calls.is_empty() || !content.is_empty() {
                    return Ok((content, calls));
                }
            }
        }
        // Fallback to OpenAI compat
        let openai_base = if base.ends_with("/v1") { base.to_string() } else { format!("{}/v1", base) };
        let url = format!("{}/chat/completions", openai_base.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "tools": tools_json,
            "tool_choice": "auto"
        });
        let resp = client.post(&url).json(&body).send().await.context("Failed to send Ollama tool chat")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama tool error {}: {}", status, text);
        }
        let json: serde_json::Value = resp.json().await.context("Failed to parse Ollama tool response")?;
        let content = json.pointer("/choices/0/message/content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut calls = Vec::new();
        if let Some(arr) = json.pointer("/choices/0/message/tool_calls").and_then(|v| v.as_array()) {
            for item in arr {
                let name = item.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if name.is_empty() { continue; }
                let args_val = item.pointer("/function/arguments");
                let (args_str, parsed) = match args_val {
                    Some(v) if v.is_string() => {
                        let s = v.as_str().unwrap_or("{}").to_string();
                        let p: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));
                        (s, p)
                    }
                    Some(v) => (v.to_string(), v.clone()),
                    None => ("{}".to_string(), serde_json::json!({})),
                };
                let id = item.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                calls.push(ToolCall { id, name, arguments: args_str, parsed_args: parsed });
            }
        }
        Ok((content, calls))
    }
}

impl OllamaProvider {
    async fn stream_native(
        &self,
        base: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        on_chunk: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<String>
    {
        let url = format!("{}/api/chat", base);
        let client = reqwest::Client::new();
        let body = OllamaChatRequest { model: model.to_string(), messages, stream: true };
        let resp = client.post(&url).json(&body).send().await.context("Failed to send Ollama /api/chat")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama /api/chat error {}: {}", status, text);
        }
        // Ollama streams NDJSON lines: {"message":{"content":"Hello"},"done":false}
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut full = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();
                if line.is_empty() { continue; }
                if let Ok(json) = serde_json::from_str::<OllamaChatChunk>(&line) {
                    if let Some(done) = json.done { if done { return Ok(full); } }
                    if let Some(content) = json.message.and_then(|m| m.content) {
                        full.push_str(&content);
                        on_chunk(&content);
                    }
                }
            }
        }
        // Flush remaining
        let trimmed = buffer.trim();
        if !trimmed.is_empty() {
            if let Ok(json) = serde_json::from_str::<OllamaChatChunk>(trimmed) {
                if let Some(content) = json.message.and_then(|m| m.content) {
                    full.push_str(&content);
                    on_chunk(&content);
                }
            }
        }
        Ok(full)
    }

    async fn stream_openai_compat(
        &self,
        base: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        on_chunk: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<String>
    {
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": 0.7,
            "max_tokens": 8192,
            "stream": true
        });
        let resp = client.post(&url).json(&body).send().await.context("Failed to send Ollama OpenAI compat chat")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama OpenAI compat error {}: {}", status, text);
        }
        // SSE same as LM Studio
        #[derive(Debug, Deserialize)]
        struct StreamChunk { choices: Option<Vec<StreamChoice>> }
        #[derive(Debug, Deserialize)]
        struct StreamChoice { delta: Option<Delta> }
        #[derive(Debug, Deserialize)]
        struct Delta { content: Option<String> }

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut full = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            let text = String::from_utf8_lossy(&bytes);
            buffer.push_str(&text);
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();
                if line.is_empty() || !line.starts_with("data:") { continue; }
                let data = line[5..].trim();
                if data == "[DONE]" { return Ok(full); }
                if let Ok(json) = serde_json::from_str::<StreamChunk>(data) {
                    if let Some(content) = json.choices.and_then(|c| c.into_iter().next()).and_then(|c| c.delta).and_then(|d| d.content) {
                        full.push_str(&content);
                        on_chunk(&content);
                    }
                }
            }
        }
        Ok(full)
    }
}
