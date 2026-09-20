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
