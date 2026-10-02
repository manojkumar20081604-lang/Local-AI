pub mod generic;
pub mod lmstudio;
pub mod ollama;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::core::config::{AppConfig, ProviderKind};
use crate::core::tools::{ToolCall, ToolDefinition};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AIModel {
    pub id: String,
    pub object: String,
    pub owned_by: String,
    /// Which provider supplied this model (for display)
    #[serde(default)]
    pub provider: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn default_url(&self) -> &str;
    async fn health_check(&self, base_url: &str) -> bool;
    async fn list_models(&self, base_url: &str) -> Result<Vec<AIModel>>;
    async fn stream_chat(
        &self,
        base_url: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        temperature: f32,
        on_chunk: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<String>;

    fn supports_tools(&self) -> bool { false }

    async fn chat_with_tools(
        &self,
        _base_url: &str,
        _model: &str,
        _messages: Vec<ChatMessage>,
        _tools: Vec<ToolDefinition>,
        _temperature: f32,
    ) -> Result<(String, Vec<ToolCall>)> {
        anyhow::bail!("Tools not supported for provider {}", self.name())
    }
}

pub fn get_provider(kind: &ProviderKind) -> Box<dyn Provider> {
    match kind {
        ProviderKind::LmStudio => Box::new(lmstudio::LmStudioProvider),
        ProviderKind::Ollama => Box::new(ollama::OllamaProvider),
        ProviderKind::LlamaCpp => Box::new(generic::LlamaCppProvider),
        ProviderKind::Generic => Box::new(generic::GenericProvider),
        ProviderKind::Auto => Box::new(generic::GenericProvider), // will autodetect before calling
    }
}

pub async fn autodetect(cfg: &AppConfig) -> (ProviderKind, String, bool) {
    // Order: ollama native -> lmstudio -> llamacpp -> generic
    // 5s timeout as in olla spec
    let order = vec![
        (ProviderKind::Ollama, cfg.providers.ollama.url.clone()),
        (ProviderKind::LmStudio, cfg.providers.lmstudio.url.clone()),
        (ProviderKind::LlamaCpp, cfg.providers.llamacpp.url.clone()),
        (ProviderKind::Generic, cfg.providers.generic.url.clone()),
    ];
    for (kind, url) in order {
        if url.is_empty() {
            continue;
        }
        let provider = get_provider(&kind);
        // Quick health check with 5s timeout
        let check = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            provider.health_check(&url),
        )
        .await
        .unwrap_or(false);
        if check {
            return (kind, url, true);
        }
    }
    // No provider up
    (ProviderKind::Auto, String::new(), false)
}

pub fn resolve_active_provider(
    cfg: &AppConfig,
    cli_provider: Option<ProviderKind>,
    cli_url: Option<&str>,
    cli_lm_studio_url: Option<&str>,
) -> (ProviderKind, String) {
    // Priority: --provider > config active > auto
    let kind = cli_provider.unwrap_or_else(|| cfg.provider.active.clone());
    let url = if let Some(u) = cli_url {
        u.to_string()
    } else if let Some(u) = cli_lm_studio_url {
        u.to_string()
    } else if let Some(u) = &cfg.provider.url {
        if !u.is_empty() { u.clone() } else { get_default_url(&kind, cfg) }
    } else {
        get_default_url(&kind, cfg)
    };
    if kind == ProviderKind::Auto && url.is_empty() {
        // Caller should autodetect; return empty and let caller handle
        return (ProviderKind::Auto, String::new());
    }
    (kind, url)
}

fn get_default_url(kind: &ProviderKind, cfg: &AppConfig) -> String {
    match kind {
        ProviderKind::LmStudio => cfg.providers.lmstudio.url.clone(),
        ProviderKind::Ollama => cfg.providers.ollama.url.clone(),
        ProviderKind::LlamaCpp => cfg.providers.llamacpp.url.clone(),
        ProviderKind::Generic => cfg.providers.generic.url.clone(),
        ProviderKind::Auto => String::new(),
    }
}

// Unified helpers used by chat/models commands

/// Pure model pick: explicit `--model` > saved `model` (when listed) >
/// first available. Offline-safe; listing happens in the wrapper below.
pub fn pick_model(
    explicit: Option<&str>,
    saved: Option<&str>,
    available: &[String],
) -> Option<String> {
    if let Some(m) = explicit {
        if !m.trim().is_empty() {
            return Some(m.to_string());
        }
    }
    if let Some(s) = saved {
        if !s.trim().is_empty() && available.iter().any(|m| m == s) {
            return Some(s.to_string());
        }
    }
    available.first().cloned()
}

/// First available model that is NOT `failed` (self-healing fallback).
/// `None` when there is nothing else to try.
pub fn fallback_model(failed: &str, available: &[String]) -> Option<String> {
    available.iter().find(|m| m.as_str() != failed).cloned()
}

/// `true` when a provider error means "this model id isn't servable"
/// (Ollama native `model 'x' not found`, compat `not_found_error`, 404s).
/// Used to skip doomed retries/fallbacks and to self-heal to another model.
pub fn is_model_not_found(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("not_found")
        || lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("model_not_found")
        || lower.contains("404")
}

/// Human-readable provider error. Extracts the server's message from
/// common JSON shapes instead of dumping raw payloads into chat.
/// Idempotent: already-friendly messages pass through unchanged.
pub fn friendly_error(err: &str, provider: &str, model: &str) -> String {
    if err.contains("isn't available here") {
        return err.chars().take(300).collect();
    }
    let message = extract_server_message(err).unwrap_or_else(|| err.to_string());
    let short: String = message.chars().take(300).collect();
    if is_model_not_found(err) {
        format!(
            "{}: model '{}' isn't available here — {} (pick a listed one: `local-ai models list`)",
            provider, model, short
        )
    } else {
        format!("{}: {}", provider, short)
    }
}

/// Pull `message` out of `{"error":{"message":…}}`, `{"error":"…"}`, or the
/// same JSON embedded in a longer line (`Ollama /api/chat error 404: {…}`).
fn extract_server_message(err: &str) -> Option<String> {
    let parse = |s: &str| -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(s).ok()?;
        v.pointer("/error/message")
            .or_else(|| v.pointer("/message"))
            .and_then(|m| m.as_str().map(|s| s.to_string()))
            .or_else(|| {
                // Plain-string error bodies: {"error": "model 'x' not found"}.
                v.get("error").and_then(|e| e.as_str()).map(|s| s.to_string())
            })
    };
    if let Some(m) = parse(err) {
        return Some(m);
    }
    // JSON embedded after a prefix — retry from the first '{'.
    err.find('{').and_then(|i| parse(&err[i..]))
}

/// Resolve the model id for a run. Explicit flags never touch the network;
/// otherwise lists once and falls back to first available (a stale saved
/// model degrades gracefully instead of failing the run).
pub async fn resolve_model_id(
    explicit: Option<String>,
    kind: &ProviderKind,
    base_url: &str,
    cfg: &AppConfig,
) -> Option<String> {
    if let Some(m) = explicit {
        if !m.trim().is_empty() {
            return Some(m);
        }
    }
    let available = list_models_unified(kind, base_url, cfg)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.id)
        .collect::<Vec<_>>();
    pick_model(None, cfg.model.as_deref(), &available)
}

pub async fn list_models_unified(
    kind: &ProviderKind,
    base_url: &str,
    cfg: &AppConfig,
) -> Result<Vec<AIModel>> {
    if kind == &ProviderKind::Auto {
        // autodetect and list from first healthy, or merge all if multiple healthy
        let mut all = Vec::new();
        for (k, url) in [
            (ProviderKind::Ollama, cfg.providers.ollama.url.clone()),
            (ProviderKind::LmStudio, cfg.providers.lmstudio.url.clone()),
            (ProviderKind::LlamaCpp, cfg.providers.llamacpp.url.clone()),
            (ProviderKind::Generic, cfg.providers.generic.url.clone()),
        ] {
            if url.is_empty() { continue; }
            let p = get_provider(&k);
            let ok = tokio::time::timeout(std::time::Duration::from_secs(2), p.health_check(&url)).await.unwrap_or(false);
            if ok {
                if let Ok(mut models) = p.list_models(&url).await {
                    for m in &mut models { m.provider = k.to_string(); }
                    all.extend(models);
                }
            }
        }
        // Deduplicate by id, keep first
        let mut seen = std::collections::HashSet::new();
        all.retain(|m| seen.insert(m.id.clone()));
        return Ok(all);
    }
    let p = get_provider(kind);
    let mut models = p.list_models(base_url).await?;
    for m in &mut models { m.provider = kind.to_string(); }
    Ok(models)
}

pub async fn chat_with_tools_unified(
    kind: &ProviderKind,
    base_url: &str,
    cfg: &AppConfig,
    model: &str,
    messages: Vec<ChatMessage>,
    tools: Vec<crate::core::tools::ToolDefinition>,
    temperature: f32,
) -> Result<(String, Vec<crate::core::tools::ToolCall>)> {
    let (k, url) = if kind == &ProviderKind::Auto {
        let (dk, du, ok) = autodetect(cfg).await;
        if !ok {
            anyhow::bail!("No provider for tools");
        }
        (dk, du)
    } else {
        (kind.clone(), base_url.to_string())
    };
    let p = get_provider(&k);
    if !p.supports_tools() {
        anyhow::bail!("Provider {} does not support tools", k);
    }
    p.chat_with_tools(&url, model, messages, tools, temperature).await
}

pub async fn stream_chat_unified(
    kind: &ProviderKind,
    base_url: &str,
    cfg: &AppConfig,
    model: &str,
    messages: Vec<ChatMessage>,
    temperature: f32,
    on_chunk: &mut (dyn for<'a> FnMut(&'a str) + Send),
) -> Result<String> {
    let (k, url) = if kind == &ProviderKind::Auto {
        // autodetect healthiest
        let (dk, du, ok) = autodetect(cfg).await;
        if !ok {
            anyhow::bail!("No local provider is running. Start LM Studio (port 1234) or Ollama (port 11434). Tried: ollama={}, lmstudio={}, llamacpp={}", cfg.providers.ollama.url, cfg.providers.lmstudio.url, cfg.providers.llamacpp.url);
        }
        (dk, du)
    } else {
        (kind.clone(), base_url.to_string())
    };
    let p = get_provider(&k);
    p.stream_chat(&url, model, messages, temperature, on_chunk).await
}
