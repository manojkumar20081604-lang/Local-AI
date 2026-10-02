//! `local-ai debug` — the self-debugging loop (Phase 1.2).
//!
//! ```text
//! edit → exec(test) → parse failure → rank files → LLM fix → apply → re-test
//! ```
//!
//! Offline-safe: without a reachable model it still runs the tests, parses
//! failures, ranks relevant files and prints a diagnosis. With a model it
//! applies `<EDIT>`/`<CREATE_FILE>` fixes (SEARCH-verified) and re-tests up
//! to `--max-attempts` times. Every applied edit goes through the approval
//! gate unless `--yes`.

use anyhow::Result;
use clap::Parser;
use console::style;

use crate::core::config::{load_config, ProviderKind};
use crate::core::{debug as core_debug, edits as core_edits, fs as core_fs};
use crate::core::{intelligence, plan as core_plan, projects, provider, verifier};

#[derive(Parser)]
pub struct DebugArgs {
    /// What to fix, e.g. "fix failing tests" (used for file ranking)
    #[arg(default_value = "fix failing tests")]
    pub query: String,

    /// Project id/name/path (default: current dir)
    #[arg(long)]
    pub project: Option<String>,

    /// Test command (default: auto-detected from Cargo.toml/package.json/…)
    #[arg(long)]
    pub test_cmd: Option<String>,

    /// Max fix attempts (default 3)
    #[arg(long, default_value = "3")]
    pub max_attempts: u32,

    /// Model id (default: first available from the provider)
    #[arg(long)]
    pub model: Option<String>,

    /// Provider override (default: config/auto)
    #[arg(long)]
    pub provider: Option<String>,

    /// Preview only: print what would run, execute nothing (plan-mode safe)
    #[arg(long)]
    pub dry_run: bool,

    /// Skip the per-attempt approval prompt before applying edits
    #[arg(long)]
    pub yes: bool,
}

const FIX_SYSTEM: &str = "You are Local AI, a precise debugging agent. Return ONLY machine-readable edit blocks for the MINIMAL fix, nothing else.\n\nFILE EDITING RULES:\n<CREATE_FILE>FILE: path CONTENT: ... </CREATE_FILE>\n<EDIT>FILE: path SEARCH: ... REPLACE: ... </EDIT>\nRules: relative paths that already exist in the project (never invent files), SEARCH must match the file byte-for-byte, smallest change that fixes the failure, no refactoring, no new dependencies.";

pub async fn handle(
    args: DebugArgs,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    let cfg = load_config().unwrap_or_default();
    let provider_kind = if let Some(p) = &args.provider {
        p.parse().unwrap_or_else(|e| { eprintln!("{}", e); std::process::exit(1) })
    } else { global_provider.clone().unwrap_or_else(|| cfg.provider.active.clone()) };
    let provider_url = crate::core::config::resolve_provider_url(&provider_kind, &cfg, global_url.as_deref(), global_lm_url.as_deref());

    let proj = projects::resolve_project(args.project)?;
    let files = core_fs::list_project_files(&proj)?;
    let test_cmd = match args.test_cmd.clone().or_else(|| core_plan::detect_test_cmd(&files)) {
        Some(c) => c,
        None => anyhow::bail!("No test command detected (no Cargo.toml/package.json/pytest config). Pass --test-cmd \"<cmd>\""),
    };
    let max_attempts = args.max_attempts.clamp(1, 10);

    if args.dry_run || crate::core::config::is_plan_mode() {
        println!("{} debug preview (no execution):", style("[dry-run]").dim());
        println!("  query: {}", args.query);
        println!("  test_cmd: {}", style(&test_cmd).cyan());
        println!("  max_attempts: {}", max_attempts);
        println!("  {} each attempt: run tests → parse failures → rank files → propose <EDIT> → approval-gated apply → re-test",
            style("loop:").dim());
        if crate::core::config::is_plan_mode() && !args.dry_run {
            println!("  {} plan mode is active — switch to build to execute", style("→").dim());
        }
        return Ok(());
    }
    crate::core::config::require_build_mode("debug")?;

    // Model resolution — failure here is NON-fatal: diagnosis still works.
    let model: Option<String> = if let Some(m) = args.model.clone() {
        Some(m)
    } else {
        match provider::list_models_unified(&provider_kind, &provider_url, &cfg).await {
            Ok(v) if !v.is_empty() => Some(v[0].id.clone()),
            _ => None,
        }
    };
    if let Some(m) = &model {
        println!("{} model: {} (provider: {})", style("debug:").dim(), m, provider_kind);
    } else {
        eprintln!("{} no model reachable — will run tests + diagnose, but cannot auto-fix", style("!").yellow());
    }

    let mut transcript: Vec<serde_json::Value> = Vec::new();
    let mut outcome = "fail".to_string();

    for attempt in 1..=max_attempts {
        println!("\n{} Attempt {}/{} — `{}`", style("━━").dim(), attempt, max_attempts, test_cmd);
        let result = core_fs::run_project_command(&proj, &test_cmd)?;
        let combined = format!("{}\n{}", result.stdout, result.stderr);
        if result.success {
            println!("{} {} tests passed (exit {})", style("✅").green().bold(),
                style(format!("attempt {}", attempt)).bold(), result.exit_code.unwrap_or(0));
            transcript.push(serde_json::json!({"attempt": attempt, "exit": result.exit_code, "result": "pass"}));
            outcome = "pass".to_string();
            break;
        }

        println!("{} exit {} — parsing failures…", style("❌ FAIL").red().bold(), result.exit_code.unwrap_or(1));
        let tail = core_debug::tail_lines(&combined, 30);
        if !tail.trim().is_empty() {
            println!("{}", style(tail.lines().take(15).collect::<Vec<_>>().join("\n")).dim());
        }
        let failures = core_debug::parse_failures(&combined);
        if failures.is_empty() {
            println!("  {} no file:line locations parsed — showing raw tail above", style("?").yellow());
        } else {
            println!("  {} parsed {} failure(s):", style("→").dim(), failures.len());
            for f in failures.iter().take(8) {
                match (&f.file, f.line) {
                    (Some(p), Some(l)) => println!("    {} {}:{} — {}", style(f.kind.as_str()).yellow(), p, l, f.message),
                    (Some(p), None) => println!("    {} {} — {}", style(f.kind.as_str()).yellow(), p, f.message),
                    _ => println!("    {} {}", style(f.kind.as_str()).yellow(), f.message),
                }
            }
        }

        // Rank files by query + failure text.
        let rank_query = format!("{} {}", args.query,
            failures.iter().map(|f| f.message.clone()).collect::<Vec<_>>().join(" "));
        let intent = intelligence::detect_intent(&rank_query);
        let ranked = intelligence::rank_relevant_files(&rank_query, &files, &intent);
        let top: Vec<String> = ranked.iter().take(5).map(|r| r.file.path.clone()).collect();
        if !top.is_empty() {
            println!("  {} relevant: {}", style("→").dim(), top.join(", "));
        }

        let Some(model_id) = model.clone() else {
            println!("\n{} diagnosis without model:", style("→").dim());
            println!("  Fix {} (or nearby) and re-run `{}`.", top.first().map(|s| s.as_str()).unwrap_or("the failing files"), test_cmd);
            transcript.push(serde_json::json!({"attempt": attempt, "exit": result.exit_code, "result": "no-model-diagnosis",
                "failures": failures, "relevant": top}));
            outcome = "no-model".to_string();
            break;
        };

        // Ask the model for a minimal fix.
        let context = intelligence::build_project_context(&rank_query, &files, |p| {
            core_fs::read_project_file(&proj, p).ok()
        });
        let mut context_capped = context;
        if context_capped.len() > 24000 {
            context_capped.truncate(24000);
            context_capped.push_str("\n…[truncated]");
        }
        let user_msg = format!(
            "Test command `{}` failed (attempt {}/{}).\n\nFAILURES:\n{}\n\nRELEVANT FILES: {}\n\nPROJECT CONTEXT:\n{}\n\nReturn ONLY the <EDIT>/<CREATE_FILE> blocks for the minimal fix.",
            test_cmd, attempt, max_attempts,
            failures.iter().map(|f| format!("- [{}] {}:{} — {}",
                f.kind, f.file.as_deref().unwrap_or("?"), f.line.map(|l| l.to_string()).unwrap_or_else(|| "?".into()), f.message))
                .collect::<Vec<_>>().join("\n"),
            top.join(", "),
            context_capped
        );
        let messages = vec![
            provider::ChatMessage { role: "system".into(), content: FIX_SYSTEM.to_string() },
            provider::ChatMessage { role: "user".into(), content: user_msg },
        ];
        println!("  {} asking {} for a fix…", style("→").dim(), model_id);
        let mut sink = |_chunk: &str| {};
        let answer = provider::stream_chat_unified(
            &provider_kind, &provider_url, &cfg, &model_id, messages, 0.2, &mut sink,
        ).await.unwrap_or_else(|e| format!("(model error: {})", e));

        // Defense in depth: verifier flags invented files, apply engine enforces.
        if !files.is_empty() {
            let report = verifier::verify_response_hybrid(&answer, &proj, &files, "strict", false);
            if !report.invented_files.is_empty() {
                eprintln!("  {} model referenced unknown files (will be refused): {}",
                    style("⚠").yellow(), report.invented_files.join(", "));
            }
        }

        let ops = core_edits::parse_edit_blocks(&answer);
        if ops.is_empty() {
            println!("  {} model returned no edit blocks. Raw answer:\n{}", style("!").yellow(), answer);
            transcript.push(serde_json::json!({"attempt": attempt, "exit": result.exit_code,
                "result": "no-edits", "failures": failures}));
            outcome = "no-edits".to_string();
            break;
        }
        println!("  {} proposed {} op(s):", style("→").dim(), ops.len());
        for op in &ops {
            println!("    {} — {}", op.kind(), op.path());
        }
        // Dry-verify first so the user sees what would be refused.
        let preview = core_edits::apply_ops(&proj, &ops, true)?;
        for (p, reason) in &preview.skipped {
            eprintln!("    {} {} — would refuse: {}", style("✗").red(), p, reason);
        }
        if preview.applied.is_empty() {
            eprintln!("  {} all ops refused — stopping (fix the SEARCH anchors or paths)", style("✗").red());
            transcript.push(serde_json::json!({"attempt": attempt, "exit": result.exit_code,
                "result": "all-refused", "failures": failures}));
            outcome = "all-refused".to_string();
            break;
        }
        if !args.yes {
            let ok = dialoguer::Confirm::new()
                .with_prompt(format!("Apply {} edit(s)?", preview.applied.len()))
                .default(false)
                .interact_opt()?;
            if !ok.unwrap_or(false) {
                println!("  {} rejected by user", style("✗").red());
                transcript.push(serde_json::json!({"attempt": attempt, "exit": result.exit_code, "result": "rejected"}));
                outcome = "rejected".to_string();
                break;
            }
        }
        let report = core_edits::apply_ops(&proj, &ops, false)?;
        for a in &report.applied {
            println!("    {} {}", style("✓").green(), a);
        }
        for (p, reason) in &report.skipped {
            eprintln!("    {} {} — {}", style("✗").red(), p, reason);
        }
        transcript.push(serde_json::json!({"attempt": attempt, "exit": result.exit_code,
            "result": "applied", "applied": report.applied, "skipped": report.skipped,
            "failures": failures, "relevant": top}));
        // Loop continues → re-test.
    }

    let report = serde_json::json!({
        "query": args.query, "test_cmd": test_cmd, "model": model,
        "max_attempts": max_attempts, "outcome": outcome, "attempts": transcript,
    });
    match save_debug_transcript(&proj, &report) {
        Ok(p) => eprintln!("\n{} transcript: {}", style("→").dim(), p.display()),
        Err(e) => eprintln!("\n{} could not save transcript: {}", style("!").yellow(), e),
    }
    if outcome == "pass" {
        println!("{} debug complete — green", style("✅").green().bold());
        Ok(())
    } else {
        anyhow::bail!("debug stopped with outcome '{}' after {} attempt(s) — see transcript", outcome, transcript.len())
    }
}

fn save_debug_transcript(proj: &projects::Project, report: &serde_json::Value) -> Result<std::path::PathBuf> {
    let base = dirs::cache_dir().ok_or_else(|| anyhow::anyhow!("No cache dir"))?;
    let dir = base.join("local-ai").join(&proj.id).join("debug");
    std::fs::create_dir_all(&dir)?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(format!("debug-{}.json", ts));
    std::fs::write(&path, serde_json::to_string_pretty(report)?)?;
    Ok(path)
}
