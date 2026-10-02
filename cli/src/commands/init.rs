//! `local-ai init` — first-run setup, on demand.
//!
//! ```bash
//! local-ai init                                          # interactive wizard
//! local-ai init --provider ollama --model qwen3-coder   # non-interactive
//! ```

use anyhow::Result;
use clap::Parser;

use crate::core::setup;

#[derive(Parser)]
pub struct InitArgs {
    /// Provider for non-interactive setup (lmstudio|ollama|llamacpp|generic)
    #[arg(long)]
    pub provider: Option<String>,

    /// Model for non-interactive setup (verified against the provider)
    #[arg(long)]
    pub model: Option<String>,
}

pub async fn handle(args: InitArgs) -> Result<()> {
    match (args.provider, args.model) {
        (None, None) => {
            setup::run_wizard().await?;
        }
        (Some(provider), model) => {
            let out = setup::run_headless(&provider, model).await?;
            println!(
                "Saved: provider={} model={} → {}",
                out.kind,
                out.model.as_deref().unwrap_or("(first available at runtime)"),
                crate::core::config::config_path()?.display()
            );
        }
        // --model without --provider: needs an endpoint to verify against.
        (None, Some(_)) => {
            anyhow::bail!("`--model` needs `--provider` (e.g. `local-ai init --provider ollama --model qwen3-coder`)");
        }
    }
    Ok(())
}

/// Bare `local-ai` launch: banner → first-run setup (once) → 3-line
/// summary → TUI. Second launches never ask again (`/model`, `/provider`
/// in the TUI or `provider select` / `models select` to change).
pub async fn launch() -> Result<()> {
    if !console::user_attended() {
        anyhow::bail!("No interactive terminal — use subcommands (e.g. `local-ai chat \"hi\"`) or `local-ai init --provider ollama --model <id>`");
    }
    if setup::is_first_run() {
        setup::run_wizard().await?;
    } else {
        print_summary().await;
    }
    super::tui::handle(super::tui::TuiArgs {
        project: None,
        no_animation: false,
        ascii: false,
        theme: None,
    })
    .await
}

/// Repeat-launch summary: provider ✓, model ✓ (or inline select), project ✓.
async fn print_summary() {
    use console::style;
    use crate::core::config::{load_config, ProviderKind};

    let mut cfg = load_config().unwrap_or_default();

    // Provider: autodetect for auto, health-check otherwise.
    let (kind, url, up) = if cfg.provider.active == ProviderKind::Auto {
        let (k, u, ok) = crate::core::provider::autodetect(&cfg).await;
        (k, u, ok)
    } else {
        let u = crate::core::config::resolve_provider_url(&cfg.provider.active, &cfg, None, None);
        let up = setup::test_connection(&cfg.provider.active, &u).await;
        (cfg.provider.active.clone(), u, up)
    };
    if up {
        println!("{} {}", style("✓").green(), kind);
    } else {
        println!("{} {} — not running.", style("!").yellow(), kind);
        println!("  Start LM Studio (:1234, enable Local Server) or Ollama (`ollama serve`), then press on — continuing offline-capable.");
    }

    // Model: saved-and-listed wins; otherwise pick inline once and save it.
    let models = if up {
        crate::core::provider::list_models_unified(&kind, &url, &cfg)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    match (&cfg.model.clone(), models.first()) {
        (Some(saved), _) if models.iter().any(|m| m == saved) => {
            println!("{} {}", style("✓").green(), saved);
        }
        _ if !models.is_empty() => {
            let labels = models.clone();
            let pick = dialoguer::Select::new()
                .with_prompt("Model (saved for next launch)")
                .items(&labels)
                .default(0)
                .interact_opt()
                .unwrap_or(None);
            if let Some(i) = pick {
                cfg.model = Some(models[i].clone());
                let _ = crate::core::config::save_config(&cfg);
                println!("{} {}", style("✓").green(), models[i]);
            } else {
                println!("{} using {} (not saved)", style("→").dim(), models[0]);
            }
        }
        _ => println!("{} no models listed", style("→").dim()),
    }

    // Project: current directory.
    match crate::core::projects::resolve_project(None) {
        Ok(proj) => {
            let name = proj.folder_path.as_ref()
                .and_then(|p| std::path::PathBuf::from(p).file_name().map(|s| s.to_string_lossy().to_string()))
                .unwrap_or(proj.name);
            println!("{} Project detected ({})", style("✓").green(), name);
        }
        Err(_) => println!("{} no project here (TUI still works — attach one with `project attach`)", style("→").dim()),
    }
    println!("\n      ◉ᴗ◉  Ready.\n");
}
