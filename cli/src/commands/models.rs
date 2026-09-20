use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::config::{AppConfig, ProviderKind, load_config};
use crate::core::provider;

#[derive(Parser)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: Option<ModelsCommands>,

    /// Provider override for this call (auto|lmstudio|ollama|llamacpp|generic)
    #[arg(long)]
    pub provider: Option<String>,

    /// Show raw JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum ModelsCommands {
    /// List available models (autodetects all providers, merges results)
    List,
    /// Check connectivity for all providers
    Status,
}

fn resolve_args_provider(local_provider: Option<String>, global_provider: Option<ProviderKind>) -> Option<ProviderKind> {
    if let Some(s) = local_provider {
        return Some(s.parse().unwrap_or_else(|e| { eprintln!("{}", e); std::process::exit(1)}));
    }
    global_provider
}

pub async fn handle(
    args: ModelsArgs,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    let cfg = load_config().unwrap_or_else(|_| AppConfig::default());
    let arg_kind = resolve_args_provider(args.provider.clone(), global_provider);
    // Determine effective provider for status display
    let effective = arg_kind.clone().unwrap_or_else(|| cfg.provider.active.clone());

    let cmd = args.command.unwrap_or(ModelsCommands::List);
    match cmd {
        ModelsCommands::List => {
            // If explicit provider + url, list from that single provider
            let (kind, url) = if arg_kind.is_some() || global_url.is_some() || global_lm_url.is_some() {
                // Single provider mode
                let k = arg_kind.clone().unwrap_or(effective.clone());
                let u = crate::core::config::resolve_provider_url(&k, &cfg, global_url.as_deref(), global_lm_url.as_deref());
                // If auto with no url, fallback to autodetect merge via provider::list_models_unified
                if k == ProviderKind::Auto && u.is_empty() {
                    // Use unified merge
                    let models = provider::list_models_unified(&ProviderKind::Auto, "", &cfg).await.unwrap_or_default();
                    print_models(models, args.json);
                    return Ok(());
                }
                (k, u)
            } else {
                // No explicit provider: merge all healthy providers (auto)
                let models = provider::list_models_unified(&ProviderKind::Auto, "", &cfg).await.unwrap_or_default();
                print_models(models, args.json);
                return Ok(());
            };
            println!("Fetching models from {} (provider={}) ...", url, kind);
            let models = provider::list_models_unified(&kind, &url, &cfg).await;
            match models {
                Ok(models) => print_models(models, args.json),
                Err(e) => {
                    eprintln!("{} {} (provider={}, url={})", style("Provider not available:").red(), e, kind, url);
                    eprintln!("  Try: local-ai doctor  or  local-ai models list --provider ollama --json");
                    std::process::exit(1);
                }
            }
        }
        ModelsCommands::Status => {
            println!("Config provider: {}  url: {}", cfg.provider.active, crate::core::config::resolve_provider_url(&cfg.provider.active, &cfg, None, None));
            // Check each provider
            for (kind, url) in [
                (ProviderKind::Ollama, cfg.providers.ollama.url.clone()),
                (ProviderKind::LmStudio, cfg.providers.lmstudio.url.clone()),
                (ProviderKind::LlamaCpp, cfg.providers.llamacpp.url.clone()),
                (ProviderKind::Generic, cfg.providers.generic.url.clone()),
            ] {
                if url.is_empty() { continue; }
                let p = provider::get_provider(&kind);
                let ok = tokio::time::timeout(std::time::Duration::from_secs(5), p.health_check(&url)).await.unwrap_or(false);
                if ok {
                    match p.list_models(&url).await {
                        Ok(models) => println!("{} {} — {} model(s)", style(format!("{}", kind)).cyan(), style("OK").green(), models.len()),
                        Err(e) => println!("{} {} — health OK but list failed: {}", style(format!("{}", kind)).cyan(), style("DEGRADED").yellow(), e),
                    }
                } else {
                    println!("{} {} — not reachable at {}", style(format!("{}", kind)).cyan(), style("UNAVAILABLE").red(), url);
                }
            }
            // Also show unified count
            let models = provider::list_models_unified(&ProviderKind::Auto, "", &cfg).await.unwrap_or_default();
            println!("\nUnified (auto) total: {} model(s) from healthy providers", models.len());
            if args.json {
                println!("{}", serde_json::to_string_pretty(&models).unwrap_or_default());
            }
        }
    }
    Ok(())
}

fn print_models(models: Vec<provider::AIModel>, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(&models).unwrap_or_default());
        return;
    }
    if models.is_empty() {
        println!("No models found. Load a model in LM Studio (port 1234) or Ollama (ollama pull llama3.1 && ollama serve).");
        return;
    }
    println!("{:<45} {:<12} {:<15} {}", "ID", "PROVIDER", "OBJECT", "OWNED_BY");
    for m in models {
        println!("{:<45} {:<12} {:<15} {}", style(&m.id).cyan(), m.provider, m.object, m.owned_by);
    }
}
