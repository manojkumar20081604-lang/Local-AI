use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Auto,
    LmStudio,
    Ollama,
    LlamaCpp,
    Generic,
}

impl Default for ProviderKind {
    fn default() -> Self { Self::Auto }
}

impl std::str::FromStr for ProviderKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "lmstudio" | "lm_studio" | "lm-studio" => Ok(Self::LmStudio),
            "ollama" => Ok(Self::Ollama),
            "llamacpp" | "llama.cpp" | "llama_cpp" => Ok(Self::LlamaCpp),
            "generic" | "openai" | "vllm" => Ok(Self::Generic),
            _ => Err(format!("unknown provider '{}', expected auto|lmstudio|ollama|llamacpp|generic", s)),
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Auto => "auto",
            Self::LmStudio => "lmstudio",
            Self::Ollama => "ollama",
            Self::LlamaCpp => "llamacpp",
            Self::Generic => "generic",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderConfig {
    pub url: String,
    #[serde(default)]
    pub use_openai_compat: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self { Self { url: String::new(), use_openai_compat: false } }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub lmstudio: ProviderConfig,
    #[serde(default)]
    pub ollama: ProviderConfig,
    #[serde(default)]
    pub llamacpp: ProviderConfig,
    #[serde(default)]
    pub generic: ProviderConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GroundingConfig {
    #[serde(default = "default_grounding_mode")]
    pub mode: String, // strict | balanced | creative
    #[serde(default = "default_true")]
    pub require_citations: bool,
}

fn default_grounding_mode() -> String { "balanced".to_string() }
fn default_true() -> bool { true }

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EmbeddingsConfig {
    #[serde(default = "default_embeddings_provider")]
    pub provider: String,
    #[serde(default = "default_embeddings_model")]
    pub model: String,
}

fn default_embeddings_provider() -> String { "tfidf".to_string() } // fastembed requires 120MB download, default to offline tfidf
fn default_embeddings_model() -> String { "BAAI/bge-small-en-v1.5".to_string() }

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AppMode {
    Plan,
    Build,
}

impl Default for AppMode {
    fn default() -> Self { Self::Build }
}

impl std::str::FromStr for AppMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "plan" => Ok(Self::Plan),
            "build" => Ok(Self::Build),
            _ => Err(format!("unknown mode '{}', expected plan|build", s)),
        }
    }
}

impl std::fmt::Display for AppMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Plan => "plan",
            Self::Build => "build",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub mode: AppMode,
    #[serde(default)]
    pub provider: ProviderSection,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub grounding: GroundingConfig,
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProviderSection {
    #[serde(default)]
    pub active: ProviderKind,
    #[serde(default)]
    pub url: Option<String>, // global override --url
}

pub fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("Could not determine config dir")?;
    let dir = if cfg!(target_os = "macos") {
        base.join("local-ai")
    } else if cfg!(target_os = "windows") {
        base.join("local-ai")
    } else {
        base.join("local-ai")
    };
    Ok(dir.join("config.toml"))
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(default_config());
    }
    let content = fs::read_to_string(&path).context("Failed to read config.toml")?;
    if content.trim().is_empty() {
        return Ok(default_config());
    }
    let cfg: AppConfig = toml::from_str(&content).context("Failed to parse config.toml")?;
    Ok(cfg)
}

pub fn save_config(cfg: &AppConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(cfg).context("Failed to serialize config")?;
    fs::write(&path, content)?;
    Ok(())
}

pub fn default_config() -> AppConfig {
    AppConfig {
        mode: AppMode::Build,
        provider: ProviderSection { active: ProviderKind::Auto, url: None },
        providers: ProvidersConfig {
            lmstudio: ProviderConfig { url: "http://localhost:1234/v1".to_string(), use_openai_compat: false },
            ollama: ProviderConfig { url: "http://localhost:11434".to_string(), use_openai_compat: false },
            llamacpp: ProviderConfig { url: "http://localhost:8080".to_string(), use_openai_compat: false },
            generic: ProviderConfig { url: String::new(), use_openai_compat: false },
        },
        grounding: GroundingConfig { mode: "balanced".to_string(), require_citations: true },
        embeddings: EmbeddingsConfig { provider: "tfidf".to_string(), model: "BAAI/bge-small-en-v1.5".to_string() },
    }
}

pub fn resolve_provider_url(kind: &ProviderKind, cfg: &AppConfig, cli_url: Option<&str>, cli_lm_studio_url: Option<&str>) -> String {
    // Priority: --url > --lm-studio-url (compat) > config provider.url > config provider-specific > default
    if let Some(u) = cli_url { return u.to_string(); }
    if let Some(u) = cli_lm_studio_url { return u.to_string(); }
    if let Some(u) = &cfg.provider.url { if !u.is_empty() { return u.clone(); } }
    match kind {
        ProviderKind::LmStudio => cfg.providers.lmstudio.url.clone(),
        ProviderKind::Ollama => cfg.providers.ollama.url.clone(),
        ProviderKind::LlamaCpp => cfg.providers.llamacpp.url.clone(),
        ProviderKind::Generic => cfg.providers.generic.url.clone(),
        ProviderKind::Auto => String::new(), // autodetect will pick
    }
}

// --- Mode helpers (plan = read-only, build = allow writes) ---

pub fn is_plan_mode() -> bool {
    // Priority: env LOCAL_AI_MODE > --mode flag (sets env) > config file > default build
    if let Ok(m) = std::env::var("LOCAL_AI_MODE") {
        if m.to_lowercase() == "plan" { return true; }
        if m.to_lowercase() == "build" { return false; }
    }
    if let Ok(cfg) = load_config() {
        if cfg.mode == AppMode::Plan { return true; }
    }
    false
}

pub fn require_build_mode(action: &str) -> anyhow::Result<()> {
    if is_plan_mode() {
        anyhow::bail!(
            "Plan mode is active — {} is disabled (read-only). Switch to build mode with:\n  local-ai --mode build {}  or  local-ai config set mode build  or  export LOCAL_AI_MODE=build",
            action, action
        );
    }
    Ok(())
}

pub fn current_mode() -> AppMode {
    if is_plan_mode() { AppMode::Plan } else { AppMode::Build }
}
