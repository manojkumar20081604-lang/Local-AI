//! First-run setup — provider select → connection → model select → save.
//!
//! ```text
//! local-ai            # first run → wizard, later runs → 3-line summary
//! local-ai init                        # wizard on demand
//! local-ai init --provider ollama --model qwen3-coder   # non-interactive
//! local-ai config set provider ollama  # script-friendly alternative
//! local-ai config set model qwen3-coder
//! ```
//!
//! Global config only (`~/.config/local-ai/config.toml`) — provider/model
//! choices are never stored inside projects.

use anyhow::Result;

use super::config::{load_config, save_config, AppConfig, ProviderKind};

/// `true` when no config file exists yet (first launch).
pub fn is_first_run() -> bool {
    match super::config::config_path() {
        Ok(p) => !p.exists(),
        Err(_) => true,
    }
}

/// One selectable provider row: kind + default endpoint + install hint.
pub struct ProviderOption {
    pub kind: ProviderKind,
    pub label: &'static str,
    pub url: String,
    pub hint: &'static str,
}

/// LM Studio / Ollama / llama.cpp with the configured (or default) URLs.
pub fn provider_options(cfg: &AppConfig) -> Vec<ProviderOption> {
    vec![
        ProviderOption {
            kind: ProviderKind::LmStudio,
            label: "LM Studio",
            url: cfg.providers.lmstudio.url.clone(),
            hint: "https://lmstudio.ai — enable Local Server (:1234)",
        },
        ProviderOption {
            kind: ProviderKind::Ollama,
            label: "Ollama",
            url: cfg.providers.ollama.url.clone(),
            hint: "https://ollama.com — ollama serve && ollama pull <model>",
        },
        ProviderOption {
            kind: ProviderKind::LlamaCpp,
            label: "llama.cpp",
            url: cfg.providers.llamacpp.url.clone(),
            hint: "llama-server on :8080",
        },
    ]
}

/// Health-check one provider (5s timeout). Pure connectivity, no models.
pub async fn test_connection(kind: &ProviderKind, url: &str) -> bool {
    if url.is_empty() {
        return false;
    }
    let p = super::provider::get_provider(kind);
    tokio::time::timeout(std::time::Duration::from_secs(5), p.health_check(url))
        .await
        .unwrap_or(false)
}

/// Live model ids for a provider (empty when unreachable).
pub async fn list_models(kind: &ProviderKind, url: &str) -> Vec<String> {
    if url.is_empty() {
        return Vec::new();
    }
    let p = super::provider::get_provider(kind);
    match p.list_models(url).await {
        Ok(models) => models.into_iter().map(|m| m.id).collect(),
        Err(_) => Vec::new(),
    }
}

/// Persist provider (+ optional model). Never touches projects.
pub fn save_selection(kind: ProviderKind, model: Option<String>) -> Result<()> {
    let mut cfg = load_config().unwrap_or_default();
    cfg.provider.active = kind;
    cfg.model = model;
    save_config(&cfg)
}

/// Result of the interactive wizard (also printed as the ready summary).
pub struct WizardResult {
    pub kind: ProviderKind,
    pub url: String,
    pub model: Option<String>,
}

fn interact_select(prompt: &str, items: &[String]) -> Result<Option<usize>> {
    Ok(dialoguer::Select::new()
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact_opt()?)
}

/// Interactive first-run wizard: banner → provider → retry loop → model → save.
/// Bails on non-interactive terminals (use `init --provider/--model` there).
pub async fn run_wizard() -> Result<WizardResult> {
    use console::style;
    if !console::user_attended() {
        anyhow::bail!("No interactive terminal — run `local-ai init --provider <ollama|lmstudio|llamacpp> [--model <id>]` instead");
    }
    println!("{}", style("╭──────────────────────────────────────────────╮").cyan().bold());
    println!("{}", style("│                   LOCAL-AI                   │").cyan().bold());
    println!("{}", style("│          Local coding agent for your CLI     │").dim());
    println!("{}", style("╰──────────────────────────────────────────────╯").cyan().bold());
    println!("\nNo provider configured. Select your local AI provider:");

    let cfg = load_config().unwrap_or_default();
    let options = provider_options(&cfg);
    let labels: Vec<String> = options
        .iter()
        .map(|o| format!("{}  ({})", o.label, o.url))
        .chain(std::iter::once("Configure later".to_string()))
        .collect();
    let Some(pick) = interact_select("Provider", &labels)? else {
        anyhow::bail!("Setup cancelled — run `local-ai init` when ready");
    };
    if pick >= options.len() {
        // Zero-config path: autodetect every launch, decide later.
        save_selection(ProviderKind::Auto, None)?;
        return Ok(WizardResult { kind: ProviderKind::Auto, url: String::new(), model: None });
    }
    let opt = &options[pick];

    // Connection loop: Retry re-checks, Back returns to provider select.
    let url = loop {
        if test_connection(&opt.kind, &opt.url).await {
            println!("{} Connected to {} ({})", style("✓").green(), opt.label, opt.url);
            break opt.url.clone();
        }
        println!(
            "{} {} was selected but isn't running.\n  Expected: {}\n  {}",
            style("⚠").yellow(),
            opt.label,
            opt.url,
            opt.hint
        );
        let actions = vec!["Retry".to_string(), "Back".to_string()];
        match interact_select("What next", &actions)? {
            Some(0) => continue,
            _ => return Box::pin(run_wizard()).await,
        }
    };

    // Model select (live list, refreshable).
    let model = loop {
        let models = list_models(&opt.kind, &url).await;
        if models.is_empty() {
            println!("{} reachable, but no models listed — pull one first, then re-run `local-ai init`", style("→").dim());
            save_selection(opt.kind.clone(), None)?;
            return Ok(WizardResult { kind: opt.kind.clone(), url: url.clone(), model: None });
        }
        let mut items = models.clone();
        items.push("Refresh list".to_string());
        match interact_select("Model", &items)? {
            Some(i) if i < models.len() => break Some(models[i].clone()),
            Some(_) => continue,
            None => anyhow::bail!("Setup cancelled — run `local-ai init` when ready"),
        }
    };
    save_selection(opt.kind.clone(), model.clone())?;

    println!("\n{} Provider: {}", style("✓").green(), opt.label);
    if let Some(m) = &model {
        println!("{} Model: {}", style("✓").green(), m);
    }
    println!("{} Connection: Local", style("✓").green());
    println!("{} Tools: Enabled", style("✓").green());
    println!("\n      ◉ᴗ◉\n\n  Local-AI is ready.\n");
    Ok(WizardResult { kind: opt.kind.clone(), url, model })
}

/// Non-interactive setup for scripts: validates, then saves.
/// Fails fast instead of saving a dead endpoint.
pub async fn run_headless(provider_name: &str, model: Option<String>) -> Result<WizardResult> {
    let kind: ProviderKind =
        provider_name.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let cfg = load_config().unwrap_or_default();
    let url = super::config::resolve_provider_url(&kind, &cfg, None, None);
    if !test_connection(&kind, &url).await {
        anyhow::bail!("{} not reachable at {} — start it first", provider_name, url);
    }
    if let Some(m) = &model {
        let models = list_models(&kind, &url).await;
        if !models.iter().any(|x| x == m) {
            anyhow::bail!("Model '{}' not found on {} (available: {})", m, provider_name, models.join(", "));
        }
    }
    let model = match model {
        Some(m) => Some(m),
        None => list_models(&kind, &url).await.into_iter().next(),
    };
    save_selection(kind.clone(), model.clone())?;
    Ok(WizardResult { kind, url, model })
}
