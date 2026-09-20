use anyhow::{Context, Result};
use futures::StreamExt;

use super::{AIModel, ChatMessage, Provider};
use crate::core::tools::{ToolCall, ToolDefinition};

pub struct LmStudioProvider;

#[derive(Debug, serde::Deserialize)]
struct ModelsResponse { data: Vec<ModelRaw> }
#[derive(Debug, serde::Deserialize)]
struct ModelRaw { id: String, object: String, owned_by: String }

#[derive(Debug, serde::Deserialize)]
struct StreamChunk { choices: Option<Vec<StreamChoice>> }
#[derive(Debug, serde::Deserialize)]
struct StreamChoice { delta: Option<Delta> }
#[derive(Debug, serde::Deserialize)]
struct Delta { content: Option<String> }

#[async_trait::async_trait]
impl Provider for LmStudioProvider {
    fn name(&self) -> &str { "lmstudio" }
    fn default_url(&self) -> &str { "http://localhost:1234/v1" }
    fn supports_tools(&self) -> bool { true }

    async fn health_check(&self, base_url: &str) -> bool {
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build();
        let Ok(c) = client else { return false };
        c.get(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    async fn list_models(&self, base_url: &str) -> Result<Vec<AIModel>> {
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await.context("Failed to connect to LM Studio")?;
        if !resp.status().is_success() {
            anyhow::bail!("LM Studio returned HTTP {}", resp.status());
        }
        let json: ModelsResponse = resp.json().await.context("Failed to parse LM Studio models")?;
        Ok(json.data.into_iter().map(|m| AIModel { id: m.id, object: m.object, owned_by: m.owned_by, provider: "lmstudio".into() }).collect())
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
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": 8192,
            "stream": true
        });
        let resp = client.post(&url).json(&body).send().await.context("Failed to send LM Studio chat")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("LM Studio error {}: {}", status, text);
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
        let trimmed = buffer.trim();
        if trimmed.starts_with("data:") {
            let data = trimmed[5..].trim();
            if data != "[DONE]" {
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
            anyhow::bail!("LM Studio tool error {}: {}", status, text);
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
