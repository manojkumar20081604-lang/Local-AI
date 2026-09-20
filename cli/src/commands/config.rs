use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::config::{AppConfig, ProviderKind, config_path, load_config, save_config};

#[derive(Parser)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Show current config (merged: file + defaults)
    Show { #[arg(long)] json: bool },
    /// Get a config value (e.g. provider.active, providers.ollama.url)
    Get { key: String },
    /// Set a config value (e.g. provider.active ollama)
    Set { key: String, value: String },
    /// Show config file path
    Path,
    /// Reset to defaults (overwrites file)
    Reset,
}

pub async fn handle(args: ConfigArgs) -> Result<()> {
    match args.command {
        ConfigCommands::Show { json } => {
            let cfg = load_config().unwrap_or_default();
            if json {
                println!("{}", serde_json::to_string_pretty(&cfg)?);
            } else {
                println!("{} {}", style("Config:").bold(), config_path()?.display());
                println!("{}", toml::to_string_pretty(&cfg)?);
            }
        }
        ConfigCommands::Get { key } => {
            let cfg = load_config().unwrap_or_default();
            let v = get_key(&cfg, &key);
            match v {
                Some(val) => println!("{}", val),
                None => {
                    eprintln!("Unknown key '{}'. Try: mode, provider.active, providers.ollama.url, providers.lmstudio.url, grounding.mode", key);
                    std::process::exit(1);
                }
            }
        }
        ConfigCommands::Set { key, value } => {
            let mut cfg = load_config().unwrap_or_default();
            set_key(&mut cfg, &key, &value)?;
            save_config(&cfg)?;
            println!("{} {} = {}  → {}", style("✓").green(), key, value, config_path()?.display());
        }
        ConfigCommands::Path => {
            println!("{}", config_path()?.display());
        }
        ConfigCommands::Reset => {
            let cfg = crate::core::config::default_config();
            save_config(&cfg)?;
            println!("{} reset to defaults at {}", style("✓").green(), config_path()?.display());
            println!("{}", toml::to_string_pretty(&cfg)?);
        }
    }
    Ok(())
}

fn get_key(cfg: &AppConfig, key: &str) -> Option<String> {
    match key {
        "mode" => Some(cfg.mode.to_string()),
        "provider.active" => Some(cfg.provider.active.to_string()),
        "provider.url" => Some(cfg.provider.url.clone().unwrap_or_default()),
        "providers.lmstudio.url" => Some(cfg.providers.lmstudio.url.clone()),
        "providers.ollama.url" => Some(cfg.providers.ollama.url.clone()),
        "providers.llamacpp.url" => Some(cfg.providers.llamacpp.url.clone()),
        "providers.generic.url" => Some(cfg.providers.generic.url.clone()),
        "grounding.mode" => Some(cfg.grounding.mode.clone()),
        "grounding.require_citations" => Some(cfg.grounding.require_citations.to_string()),
        "embeddings.provider" => Some(cfg.embeddings.provider.clone()),
        "embeddings.model" => Some(cfg.embeddings.model.clone()),
        _ => None,
    }
}

fn set_key(cfg: &mut AppConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "mode" => {
            let m: crate::core::config::AppMode = value.parse().map_err(|e: String| anyhow::anyhow!(e))?;
            cfg.mode = m;
        }
        "provider.active" => {
            let k: ProviderKind = value.parse().map_err(|e: String| anyhow::anyhow!(e))?;
            cfg.provider.active = k;
        }
        "provider.url" => {
            cfg.provider.url = if value.is_empty() { None } else { Some(value.to_string()) };
        }
        "providers.lmstudio.url" => cfg.providers.lmstudio.url = value.to_string(),
        "providers.ollama.url" => cfg.providers.ollama.url = value.to_string(),
        "providers.llamacpp.url" => cfg.providers.llamacpp.url = value.to_string(),
        "providers.generic.url" => cfg.providers.generic.url = value.to_string(),
        "grounding.mode" => {
            if !["strict","balanced","creative"].contains(&value) {
                anyhow::bail!("grounding.mode must be strict|balanced|creative");
            }
            cfg.grounding.mode = value.to_string();
        }
        "grounding.require_citations" => {
            cfg.grounding.require_citations = value.parse().map_err(|_| anyhow::anyhow!("expected true|false"))?;
        }
        "embeddings.provider" => cfg.embeddings.provider = value.to_string(),
        "embeddings.model" => cfg.embeddings.model = value.to_string(),
        _ => anyhow::bail!("Unknown key '{}'. Valid: mode, provider.active, provider.url, providers.{{lmstudio,ollama,llamacpp,generic}}.url, grounding.mode, grounding.require_citations, embeddings.*", key),
    }
    Ok(())
}
