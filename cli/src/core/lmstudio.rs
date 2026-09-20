use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::lm_studio_base_url;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AIModel {
    pub id: String,
    pub object: String,
    pub owned_by: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<AIModel>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Option<Vec<StreamChoice>>,
}
#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Option<Delta>,
}
#[derive(Debug, Deserialize)]
struct Delta {
    content: Option<String>,
}

pub async fn get_models() -> Result<Vec<AIModel>> {
    let url = format!("{}/models", lm_studio_base_url());
    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await.context("Failed to connect to LM Studio")?;
    if !resp.status().is_success() {
        anyhow::bail!("LM Studio returned HTTP {}", resp.status());
    }
    let json: ModelsResponse = resp.json().await.context("Failed to parse models")?;
    Ok(json.data)
}

pub async fn stream_chat(
    model: &str,
    messages: Vec<ChatMessage>,
    mut on_chunk: impl FnMut(&str),
) -> Result<String> {
    let url = format!("{}/chat/completions", lm_studio_base_url());
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": 0.7,
        "max_tokens": 8192,
        "stream": true
    });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Failed to send chat request")?;
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
        // Process lines
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim().to_string();
            buffer = buffer[pos + 1..].to_string();
            if line.is_empty() || !line.starts_with("data:") {
                continue;
            }
            let data = line[5..].trim();
            if data == "[DONE]" {
                return Ok(full);
            }
            if let Ok(json) = serde_json::from_str::<StreamChunk>(data) {
                if let Some(content) = json
                    .choices
                    .and_then(|c| c.into_iter().next())
                    .and_then(|c| c.delta)
                    .and_then(|d| d.content)
                {
                    full.push_str(&content);
                    on_chunk(&content);
                }
            }
        }
    }
    // Flush remaining buffer
    let trimmed = buffer.trim();
    if trimmed.starts_with("data:") {
        let data = trimmed[5..].trim();
        if data != "[DONE]" {
            if let Ok(json) = serde_json::from_str::<StreamChunk>(data) {
                if let Some(content) = json
                    .choices
                    .and_then(|c| c.into_iter().next())
                    .and_then(|c| c.delta)
                    .and_then(|d| d.content)
                {
                    full.push_str(&content);
                    on_chunk(&content);
                }
            }
        }
    }
    Ok(full)
}

pub async fn chat_once(model: &str, messages: Vec<ChatMessage>) -> Result<String> {
    // Non-streaming fallback
    let url = format!("{}/chat/completions", lm_studio_base_url());
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": 0.7,
        "max_tokens": 8192,
        "stream": false
    });
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("LM Studio error {}", resp.status());
    }
    let json: serde_json::Value = resp.json().await?;
    let content = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .context("No content in response")?;
    Ok(content.to_string())
}
