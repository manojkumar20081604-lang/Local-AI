pub mod config;
pub mod embeddings;
pub mod fs;
pub mod index;
pub mod intelligence;
pub mod projects;
pub mod provider;
pub mod tools;
pub mod verifier;

// Keep old module for backwards compat — re-export as provider::lmstudio
pub mod lmstudio {
    pub use super::provider::lmstudio::*;
    pub use super::provider::{AIModel, ChatMessage};
}

pub fn lm_studio_base_url() -> String {
    // Backwards compat: check env first, then config
    if let Ok(url) = std::env::var("LOCAL_AI_LM_STUDIO_URL") {
        if !url.is_empty() { return url; }
    }
    if let Ok(url) = std::env::var("LOCAL_AI_URL") {
        if !url.is_empty() { return url; }
    }
    // Try config file
    if let Ok(cfg) = config::load_config() {
        if let Some(url) = cfg.provider.url.clone() {
            if !url.is_empty() { return url; }
        }
        // Return active provider url
        let kind = cfg.provider.active.clone();
        let url = match kind {
            config::ProviderKind::LmStudio => cfg.providers.lmstudio.url.clone(),
            config::ProviderKind::Ollama => cfg.providers.ollama.url.clone(),
            config::ProviderKind::LlamaCpp => cfg.providers.llamacpp.url.clone(),
            config::ProviderKind::Generic => cfg.providers.generic.url.clone(),
            config::ProviderKind::Auto => {
                // For auto, prefer lmstudio url if exists, else ollama
                if !cfg.providers.lmstudio.url.is_empty() { cfg.providers.lmstudio.url.clone() }
                else if !cfg.providers.ollama.url.is_empty() { cfg.providers.ollama.url.clone() }
                else { "http://localhost:1234/v1".to_string() }
            }
        };
        if !url.is_empty() { return url; }
    }
    "http://localhost:1234/v1".to_string()
}
