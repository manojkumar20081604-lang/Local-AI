use anyhow::Result;
use clap::Parser;
use console::style;

use crate::core::config::{AppConfig, ProviderKind, load_config};
use crate::core::provider;

#[derive(Parser)]
pub struct DoctorArgs {
    /// Show JSON output
    #[arg(long)]
    pub json: bool,
}

pub async fn handle(args: DoctorArgs) -> Result<()> {
    let cfg = load_config().unwrap_or_else(|_| AppConfig::default());

    println!("{}", style("Local AI — Doctor").bold());
    let effective_mode = crate::core::config::current_mode();
    println!("Mode: {} (config: {})  Config: {}  Grounding: {}  Embeddings: {} ({})",
        effective_mode, cfg.mode, cfg.provider.active, cfg.grounding.mode, cfg.embeddings.provider, cfg.embeddings.model);
    if crate::core::config::is_plan_mode() {
        println!("{} Plan mode ACTIVE — writes/exec disabled (read-only). Switch: --mode build or config set mode build", style("⚠").yellow());
    }
    println!("Config file: {}", crate::core::config::config_path().map(|p| p.display().to_string()).unwrap_or_else(|_| "-".into()));
    println!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);

    // GPU
    let nvidia = std::process::Command::new("nvidia-smi").arg("--query-gpu=name,memory.total").arg("--format=csv,noheader").output();
    if let Ok(out) = nvidia {
        if out.status.success() {
            println!("GPU (NVIDIA): {}", String::from_utf8_lossy(&out.stdout).trim());
        } else {
            println!("GPU (NVIDIA): not detected");
        }
    } else {
        println!("GPU (NVIDIA): not detected");
    }
    if cfg!(target_os = "macos") {
        println!("GPU (Apple): MPS available — recommend MLX + torchtune");
    }

    let mut results = Vec::new();
    let providers = vec![
        (ProviderKind::Ollama, cfg.providers.ollama.url.clone()),
        (ProviderKind::LmStudio, cfg.providers.lmstudio.url.clone()),
        (ProviderKind::LlamaCpp, cfg.providers.llamacpp.url.clone()),
        (ProviderKind::Generic, cfg.providers.generic.url.clone()),
    ];

    for (kind, url) in providers {
        if url.is_empty() {
            println!("{} {} — not configured", style(format!("{:8}", kind.to_string())).cyan(), style("SKIP").dim());
            continue;
        }
        let p = provider::get_provider(&kind);
        let ok = tokio::time::timeout(std::time::Duration::from_secs(5), p.health_check(&url)).await.unwrap_or(false);
        if ok {
            let models = p.list_models(&url).await;
            match models {
                Ok(models) => {
                    println!("{} {} — {} model(s) at {}", style(format!("{:8}", kind.to_string())).cyan(), style("OK").green(), models.len(), url);
                    for m in models.iter().take(5) {
                        println!("  - {}", m.id);
                    }
                    if models.len() > 5 { println!("  ... and {} more", models.len() - 5); }
                    results.push(serde_json::json!({"provider": kind.to_string(), "url": url, "status":"ok", "models": models.len()}));
                }
                Err(e) => {
                    println!("{} {} — health OK but list failed: {} ({})", style(format!("{:8}", kind.to_string())).cyan(), style("DEGRADED").yellow(), e, url);
                    results.push(serde_json::json!({"provider": kind.to_string(), "url": url, "status":"degraded", "error": e.to_string()}));
                }
            }
        } else {
            println!("{} {} — not reachable at {}", style(format!("{:8}", kind.to_string())).cyan(), style("UNAVAILABLE").red(), url);
            results.push(serde_json::json!({"provider": kind.to_string(), "url": url, "status":"unavailable"}));
        }
    }

    // Autodetect
    let (k, u, ok) = provider::autodetect(&cfg).await;
    if ok {
        println!("\n{} autodetected: {} at {}", style("→").cyan(), k, u);
    } else {
        println!("\n{} no provider healthy — start LM Studio (:1234) or Ollama (:11434)", style("→").yellow());
        println!("  LM Studio: https://lmstudio.ai  (enable Local Server)");
        println!("  Ollama:    https://ollama.com  (ollama serve && ollama pull llama3.1:8b)");
    }

    // Python — cross-platform (Windows: python/py, Unix: python3/python)
    let py_candidates: &[&str] = if cfg!(target_os = "windows") {
        &["python", "py", "python3"]
    } else {
        &["python3", "python"]
    };
    let mut py_found: Option<std::process::Output> = None;
    for cand in py_candidates {
        if let Ok(o) = std::process::Command::new(*cand).arg("--version").output() {
            if o.status.success() {
                py_found = Some(o);
                break;
            }
        }
    }
    if let Some(o) = py_found {
        let ver = if !o.stdout.is_empty() {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        } else {
            String::from_utf8_lossy(&o.stderr).trim().to_string()
        };
        println!("Python: {}", ver);
    } else {
        println!("Python: not found");
    }
    // Config URLs
    println!("Providers: lmstudio={}  ollama={}  llamacpp={}  generic={}",
        cfg.providers.lmstudio.url, cfg.providers.ollama.url, cfg.providers.llamacpp.url,
        if cfg.providers.generic.url.is_empty() { "(none)" } else { &cfg.providers.generic.url });

    if args.json {
        let out = serde_json::json!({
            "config": cfg,
            "autodetect": {"provider": k.to_string(), "url": u, "ok": ok},
            "providers": results
        });
        println!("\n{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    }

    println!("\nQuick start: local-ai models list  |  local-ai chat \"explain this repo\" --provider auto");
    Ok(())
}
