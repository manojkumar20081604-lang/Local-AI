//! `local-ai provider` — list, switch, and test model providers.
//!
//! ```bash
//! local-ai provider list                    # kinds + endpoints + reachability
//! local-ai provider select                  # interactive switch (saves active)
//! local-ai provider select --name ollama    # non-interactive switch
//! local-ai provider test --name lmstudio    # connectivity check
//! ```

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::config::{load_config, save_config, ProviderKind};
use crate::core::setup;

#[derive(Parser)]
pub struct ProviderArgs {
    #[command(subcommand)]
    pub command: ProviderCommands,
}

#[derive(Subcommand)]
pub enum ProviderCommands {
    /// List providers with endpoints and reachability
    List,
    /// Switch the active provider (saves to config)
    Select {
        /// Provider name (omit for interactive picker)
        #[arg(long)]
        name: Option<String>,
    },
    /// Test connectivity for one provider (default: active)
    Test {
        /// Provider name (default: active)
        #[arg(long)]
        name: Option<String>,
    },
}

pub async fn handle(args: ProviderArgs) -> Result<()> {
    match args.command {
        ProviderCommands::List => handle_list().await,
        ProviderCommands::Select { name } => handle_select(name).await,
        ProviderCommands::Test { name } => handle_test(name).await,
    }
}

async fn handle_list() -> Result<()> {
    let cfg = load_config().unwrap_or_default();
    println!("{}", style("Providers (active marked with *):").bold());
    // Reachability per row (5s each, same budget as `doctor`).
    let mut options = setup::provider_options(&cfg);
    options.push(setup::ProviderOption {
        kind: ProviderKind::Auto,
        label: "Auto (detect every launch)",
        url: String::new(),
        hint: "",
    });
    for opt in &options {
        let active = if opt.kind == cfg.provider.active { "*" } else { " " };
        if opt.url.is_empty() {
            println!("{} {:28} {}", active, opt.label, style("no endpoint").dim());
            continue;
        }
        let ok = setup::test_connection(&opt.kind, &opt.url).await;
        println!(
            "{} {:28} {} {}",
            active,
            opt.label,
            opt.url,
            if ok { style("reachable").green().to_string() } else { style("down").red().to_string() }
        );
    }
    if let Some(m) = &cfg.model {
        println!("\nSaved model: {} (change: `local-ai models select`)", style(m).cyan());
    }
    Ok(())
}

async fn active_kind(name: Option<String>) -> Result<ProviderKind> {
    if let Some(n) = name {
        return n.parse().map_err(|e: String| anyhow::anyhow!(e));
    }
    if !console::user_attended() {
        anyhow::bail!("No interactive terminal — pass `--name <lmstudio|ollama|llamacpp|generic|auto>`");
    }
    let cfg = load_config().unwrap_or_default();
    let options = setup::provider_options(&cfg);
    let labels: Vec<String> = options.iter().map(|o| o.label.to_string()).collect();
    match dialoguer::Select::new().with_prompt("Provider").items(&labels).default(0).interact_opt()? {
        Some(i) => Ok(options[i].kind.clone()),
        None => anyhow::bail!("Selection cancelled"),
    }
}

async fn handle_select(name: Option<String>) -> Result<()> {
    let kind = active_kind(name).await?;
    let mut cfg = load_config().unwrap_or_default();
    cfg.provider.active = kind.clone();
    // A saved model from another provider may not exist here — re-verify.
    if let Some(saved) = cfg.model.clone() {
        let url = crate::core::config::resolve_provider_url(&kind, &cfg, None, None);
        let models = setup::list_models(&kind, &url).await;
        if !models.iter().any(|m| m == &saved) {
            println!(
                "{} saved model '{}' not on {} — clearing (pick one: `local-ai models select`)",
                style("!").yellow(),
                saved,
                kind
            );
            cfg.model = None;
        }
    }
    let url = crate::core::config::resolve_provider_url(&kind, &cfg, None, None);
    save_config(&cfg)?;
    println!("{} active provider → {} ({})", style("✓").green(), kind, if url.is_empty() { "autodetect" } else { &url });
    Ok(())
}

async fn handle_test(name: Option<String>) -> Result<()> {
    let cfg = load_config().unwrap_or_default();
    let kind = match name {
        Some(n) => n.parse().map_err(|e: String| anyhow::anyhow!(e))?,
        None => cfg.provider.active.clone(),
    };
    let url = crate::core::config::resolve_provider_url(&kind, &cfg, None, None);
    if url.is_empty() {
        println!("{} {} has no endpoint (autodetect mode)", style("→").dim(), kind);
        return Ok(());
    }
    if setup::test_connection(&kind, &url).await {
        let models = setup::list_models(&kind, &url).await;
        println!("{} {} reachable at {} ({} model(s))", style("✓").green(), kind, url, models.len());
        for m in models.iter().take(8) {
            println!("  - {}", m);
        }
    } else {
        anyhow::bail!("{} not reachable at {} — start it first", kind, url);
    }
    Ok(())
}
