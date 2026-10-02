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

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

pub async fn handle(args: DoctorArgs) -> Result<()> {
    let cfg = load_config().unwrap_or_else(|_| AppConfig::default());
    let mut checks: Vec<Check> = Vec::new();

    // 1. Installation — the running binary exists.
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".into());
    checks.push(Check {
        name: "Installation",
        ok: !exe.contains("unknown"),
        detail: exe.clone(),
    });

    // 2. PATH — `local-ai` resolves without a relative path.
    let on_path = std::env::var_os("PATH").map(|paths| {
        std::env::split_paths(&paths).any(|d| {
            d.join(format!("local-ai{}", std::env::consts::EXE_SUFFIX)).exists()
        })
    }).unwrap_or(false);
    checks.push(Check {
        name: "PATH",
        ok: on_path,
        detail: if on_path {
            "local-ai resolves globally".into()
        } else {
            "not on PATH — run ./install.sh, then restart the terminal".into()
        },
    });

    // 3. Configuration — file exists and parses.
    let cfg_status = match crate::core::config::config_path() {
        Ok(p) if p.exists() => (true, format!("{} (first run done)", p.display())),
        Ok(p) => (false, format!("missing — run `local-ai init` ({})", p.display())),
        Err(e) => (false, e.to_string()),
    };
    checks.push(Check { name: "Configuration", ok: cfg_status.0, detail: cfg_status.1 });

    // 4. Provider — the active provider answers a health check.
    let active = cfg.provider.active.clone();
    let active_url = crate::core::config::resolve_provider_url(&active, &cfg, None, None);
    let (prov_ok, prov_detail) = if active == ProviderKind::Auto {
        let (k, u, ok) = provider::autodetect(&cfg).await;
        (
            ok,
            if ok {
                format!("autodetected {} at {}", k, u)
            } else {
                "no provider running — start LM Studio (:1234) or Ollama (:11434)".into()
            },
        )
    } else if active_url.is_empty() {
        (false, format!("{} has no endpoint configured", active))
    } else {
        let p = provider::get_provider(&active);
        let ok = tokio::time::timeout(std::time::Duration::from_secs(5), p.health_check(&active_url))
            .await
            .unwrap_or(false);
        (
            ok,
            if ok {
                format!("{} at {}", active, active_url)
            } else {
                format!("{} not reachable at {} — start it first", active, active_url)
            },
        )
    };
    checks.push(Check { name: "Provider", ok: prov_ok, detail: prov_detail });

    // 5. Model — saved model listed, else first available, else none.
    let (model_ok, model_detail, model_count) = if prov_ok {
        let (k, u) = if active == ProviderKind::Auto {
            let (k, u, _) = provider::autodetect(&cfg).await;
            (k, u)
        } else {
            (active.clone(), active_url.clone())
        };
        let models = provider::list_models_unified(&k, &u, &cfg).await.unwrap_or_default();
        match (&cfg.model, models.first()) {
            (Some(saved), _) if models.iter().any(|m| &m.id == saved) => {
                (true, format!("saved model '{}' available", saved), models.len())
            }
            (Some(saved), Some(first)) => (
                false,
                format!("saved model '{}' missing — first available is '{}' (`local-ai models select`)", saved, first.id),
                models.len(),
            ),
            (Some(saved), None) => (false, format!("saved model '{}' unreachable", saved), 0),
            (None, Some(first)) => (true, format!("no saved model — would use '{}' (`local-ai models select` to pin)", first.id), models.len()),
            (None, None) => (false, "no models listed".into(), 0),
        }
    } else {
        (false, "skipped (provider down)".into(), 0)
    };
    checks.push(Check { name: "Model", ok: model_ok, detail: model_detail });

    // 6. Project access — the current directory resolves and lists.
    let (proj_ok, proj_detail) = match crate::core::projects::resolve_project(None) {
        Ok(proj) => match crate::core::fs::list_project_files(&proj) {
            Ok(files) => {
                let n = files.iter().filter(|f| !f.is_directory).count();
                (true, format!("{} ({} files)", proj.folder_path.as_deref().unwrap_or("."), n))
            }
            Err(e) => (false, format!("unreadable: {}", e)),
        },
        Err(e) => (false, format!("unresolvable: {}", e)),
    };
    checks.push(Check { name: "Project access", ok: proj_ok, detail: proj_detail });

    // 7. Git — installed, and whether cwd is a repo.
    let (git_ok, git_detail) = match std::process::Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => {
            let ver = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let in_repo = std::process::Command::new("git")
                .args(["rev-parse", "--is-inside-work-tree"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            (true, format!("{} ({})", ver, if in_repo { "inside a repo" } else { "not a repo here" }))
        }
        _ => (false, "git not found on PATH".into()),
    };
    checks.push(Check { name: "Git", ok: git_ok, detail: git_detail });

    // 8. Terminal — interactive + UTF-8 (the TUI needs both).
    // Informational only: a pipe is expected here, not broken.
    let lang = std::env::var("LANG").or_else(|_| std::env::var("LC_ALL")).unwrap_or_default();
    let term_ok = console::user_attended();
    checks.push(Check {
        name: "Terminal",
        ok: true,
        detail: if term_ok {
            format!("interactive{} (LANG={})", if lang.to_uppercase().contains("UTF") || lang.is_empty() { ", utf-8" } else { ", no utf-8 — pass --ascii to the TUI" }, if lang.is_empty() { "unset" } else { &lang })
        } else {
            "non-interactive — TUI unavailable, subcommands still work".into()
        },
    });

    // 9. Tool system — built-ins load (filesystem/terminal/git/browser).
    let n_tools = crate::core::tools::project_tools().len();
    checks.push(Check {
        name: "Tool system",
        ok: n_tools > 0,
        detail: format!("{} built-in tools (list/read/search/exec/web_fetch…)", n_tools),
    });

    // --- Report ---
    println!("{}", style("LOCAL-AI DOCTOR").bold());
    let mut failed = 0;
    for c in &checks {
        println!(
            "{} {:<14} {}",
            if c.ok { style("✓").green().to_string() } else { style("✗").red().to_string() },
            c.name,
            if c.ok { style(&c.detail).dim().to_string() } else { style(&c.detail).yellow().to_string() }
        );
        if !c.ok {
            failed += 1;
        }
    }
    if failed == 0 {
        println!("\n{}", style("Everything looks good.").green().bold());
    } else {
        println!("\n{} {} check(s) need attention (see above)", style("→").yellow(), failed);
    }

    // --- Existing detail: per-provider health + models (unchanged behavior) ---
    println!("\n{}", style("Providers:").bold());
    for (kind, url) in [
        (ProviderKind::Ollama, cfg.providers.ollama.url.clone()),
        (ProviderKind::LmStudio, cfg.providers.lmstudio.url.clone()),
        (ProviderKind::LlamaCpp, cfg.providers.llamacpp.url.clone()),
        (ProviderKind::Generic, cfg.providers.generic.url.clone()),
    ] {
        if url.is_empty() {
            println!("{} {} — not configured", style(format!("{:8}", kind.to_string())).cyan(), style("SKIP").dim());
            continue;
        }
        let p = provider::get_provider(&kind);
        let ok = tokio::time::timeout(std::time::Duration::from_secs(5), p.health_check(&url)).await.unwrap_or(false);
        if ok {
            match p.list_models(&url).await {
                Ok(models) => {
                    println!("{} {} — {} model(s) at {}", style(format!("{:8}", kind.to_string())).cyan(), style("OK").green(), models.len(), url);
                    for m in models.iter().take(5) {
                        println!("  - {}", m.id);
                    }
                    if models.len() > 5 {
                        println!("  ... and {} more", models.len() - 5);
                    }
                }
                Err(e) => println!("{} {} — health OK but list failed: {}", style(format!("{:8}", kind.to_string())).cyan(), style("DEGRADED").yellow(), e),
            }
        } else {
            println!("{} {} — not reachable at {}", style(format!("{:8}", kind.to_string())).cyan(), style("UNAVAILABLE").red(), url);
        }
    }
    println!("\nOS: {} {}  Config: {}  Mode: {}  Models reachable: {}",
        std::env::consts::OS, std::env::consts::ARCH,
        crate::core::config::config_path().map(|p| p.display().to_string()).unwrap_or_else(|_| "-".into()),
        crate::core::config::current_mode(), model_count);

    if args.json {
        let out = serde_json::json!({
            "checks": checks.iter().map(|c| serde_json::json!({"name": c.name, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
            "config": cfg,
        });
        println!("\n{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
