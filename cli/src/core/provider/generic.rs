use anyhow::{Context, Result};
use futures::StreamExt;

use super::{AIModel, ChatMessage, Provider};
use crate::core::tools::{ToolCall, ToolDefinition};

pub struct GenericProvider;
pub struct LlamaCppProvider;

#[derive(Debug, serde::Deserialize)]
struct ModelsResponse { data: Vec<ModelRaw> }
#[derive(Debug, serde::Deserialize)]
struct ModelRaw { id: String, object: String, owned_by: Option<String> }

#[derive(Debug, serde::Deserialize)]
struct StreamChunk { choices: Option<Vec<StreamChoice>> }
#[derive(Debug, serde::Deserialize)]
struct StreamChoice { delta: Option<Delta> }
#[derive(Debug, serde::Deserialize)]
struct Delta { content: Option<String> }

#[async_trait::async_trait]
impl Provider for GenericProvider {
    fn name(&self) -> &str { "generic" }
    fn default_url(&self) -> &str { "http://localhost:8080/v1" }
    fn supports_tools(&self) -> bool { true }

    async fn health_check(&self, base_url: &str) -> bool {
        if base_url.is_empty() { return false; }
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build();
        let Ok(c) = client else { return false };
        c.get(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    async fn list_models(&self, base_url: &str) -> Result<Vec<AIModel>> {
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await.context("Failed to connect to generic OpenAI provider")?;
        if !resp.status().is_success() {
            anyhow::bail!("Provider returned HTTP {}", resp.status());
        }
        let json: ModelsResponse = resp.json().await.context("Failed to parse models")?;
        Ok(json.data.into_iter().map(|m| AIModel {
            id: m.id,
            object: m.object,
            owned_by: m.owned_by.unwrap_or_else(|| "generic".into()),
            provider: "generic".into(),
        }).collect())
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
        // Generic OpenAI streaming (reuse LM Studio logic)
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": 8192,
            "stream": true
        });
        let resp = client.post(&url).json(&body).send().await.context("Failed to send chat")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Provider error {}: {}", status, text);
        }
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

    async fn chat_with_tools(
        &self,
        base_url: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        temperature: f32,
    ) -> Result<(String, Vec<ToolCall>)> {
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let tools_json = serde_json::to_value(&tools).unwrap_or(serde_json::json!([]));
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "tools": tools_json,
            "tool_choice": "auto"
        });
        let resp = client.post(&url).json(&body).send().await.context("Failed to send tool chat")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Generic tool error {}: {}", status, text);
        }
        let json: serde_json::Value = resp.json().await.context("Failed to parse tool response")?;
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

#[async_trait::async_trait]
impl Provider for LlamaCppProvider {
    fn name(&self) -> &str { "llamacpp" }
    fn default_url(&self) -> &str { "http://localhost:8080" }

    async fn health_check(&self, base_url: &str) -> bool {
        let base = base_url.trim_end_matches('/');
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build();
        let Ok(c) = client else { return false };
        // llama.cpp: /health or /props
        if c.get(format!("{}/health", base)).send().await.map(|r| r.status().is_success()).unwrap_or(false) {
            return true;
        }
        if c.get(format!("{}/props", base)).send().await.map(|r| r.status().is_success()).unwrap_or(false) {
            return true;
        }
        // Try OpenAI compat
        c.get(format!("{}/v1/models", base)).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    async fn list_models(&self, base_url: &str) -> Result<Vec<AIModel>> {
        // llama.cpp typically serves single model, but try OpenAI endpoint
        let base = base_url.trim_end_matches('/');
        let client = reqwest::Client::new();
        let urls = vec![format!("{}/v1/models", base), format!("{}/models", base)];
        for url in urls {
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(json) = resp.json::<ModelsResponse>().await {
                        return Ok(json.data.into_iter().map(|m| AIModel {
                            id: m.id,
                            object: m.object,
                            owned_by: m.owned_by.unwrap_or_else(|| "llamacpp".into()),
                            provider: "llamacpp".into(),
                        }).collect());
                    }
                }
            }
        }
        // Fallback: query /props for model name
        let resp = client.get(format!("{}/props", base)).send().await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                if let Ok(json) = r.json::<serde_json::Value>().await {
                    if let Some(model) = json.get("model_path").and_then(|v| v.as_str()) {
                        let name = std::path::Path::new(model).file_stem().unwrap_or_default().to_string_lossy().to_string();
                        return Ok(vec![AIModel { id: if name.is_empty() { "llamacpp".into() } else { name }, object: "model".into(), owned_by: "llamacpp".into(), provider: "llamacpp".into() }]);
                    }
                }
            }
        }
        anyhow::bail!("Could not list models from llama.cpp at {}", base)
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
        // Try OpenAI compat first (llama.cpp --jinja)
        let base = base_url.trim_end_matches('/');
        let openai_bases = vec![format!("{}/v1", base), base.to_string()];
        for ob in openai_bases {
            let provider = GenericProvider;
            if let Ok(s) = provider.stream_chat(&ob, model, messages.clone(), temperature, &mut *on_chunk).await {
                return Ok(s);
            }
        }
        anyhow::bail!("llama.cpp chat not available — ensure server started with --jinja and OpenAI compat")
    }
}
