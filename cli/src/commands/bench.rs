//! `local-ai bench` — benchmark studio (Phase 5.1).
//!
//! ```bash
//! local-ai bench run --model qwen2.5:9b --suite coding --project MyApp
//! local-ai bench run --model X --prompt-set bench/prompts.jsonl --out ./bench/results
//! local-ai bench compare bench/results/a.json bench/results/b.json
//! ```
//!
//! Each prompt streams against the local model (TTFT + TPS measured) and is
//! scored pass/fail; the run file feeds `bench compare` and the
//! `finetune eval` deploy gate.

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

use crate::core::config::{load_config, ProviderKind};
use crate::core::{bench as core_bench, fs as core_fs, intelligence, projects, provider, verifier};

#[derive(Parser)]
pub struct BenchArgs {
    #[command(subcommand)]
    pub command: BenchCommands,
}

#[derive(Subcommand)]
pub enum BenchCommands {
    /// Run a prompt set against a model (streams, measures, scores, saves)
    Run {
        /// Model id (default: first available)
        #[arg(long)]
        model: Option<String>,
        /// Suite filter: coding|reasoning (default: all)
        #[arg(long)]
        suite: Option<String>,
        /// Prompt set (default: ./bench/prompts.jsonl)
        #[arg(long, default_value = "bench/prompts.jsonl")]
        prompt_set: PathBuf,
        /// Project for context + verifier (recommended)
        #[arg(long)]
        project: Option<String>,
        /// Results dir (default: ./bench/results)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Provider override
        #[arg(long)]
        provider: Option<String>,
    },
    /// Compare two result files (before/after table)
    Compare {
        /// Baseline result JSON (e.g. base model)
        a: PathBuf,
        /// Challenger result JSON (e.g. adapter)
        b: PathBuf,
    },
}

pub async fn handle(
    args: BenchArgs,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    match args.command {
        BenchCommands::Run { model, suite, prompt_set, project, out, provider } => {
            handle_run(model, suite, prompt_set, project, out, provider, global_provider, global_url, global_lm_url).await
        }
        BenchCommands::Compare { a, b } => handle_compare(a, b).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_run(
    model: Option<String>,
    suite: Option<String>,
    prompt_set: PathBuf,
    project: Option<String>,
    out: Option<PathBuf>,
    provider: Option<String>,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    let cfg = load_config().unwrap_or_default();
    let provider_kind = if let Some(p) = &provider {
        p.parse().unwrap_or_else(|e| {
            eprintln!("{}", e);
            std::process::exit(1)
        })
    } else {
        global_provider.clone().unwrap_or_else(|| cfg.provider.active.clone())
    };
    let provider_url = crate::core::config::resolve_provider_url(&provider_kind, &cfg, global_url.as_deref(), global_lm_url.as_deref());

    let prompts = core_bench::load_prompts(&prompt_set)?;
    let prompts: Vec<_> = match &suite {
        Some(s) => prompts.into_iter().filter(|p| p.suite == *s).collect(),
        None => prompts,
    };
    if prompts.is_empty() {
        anyhow::bail!("No prompts match suite {:?}", suite);
    }

    let model_id = match model {
        Some(m) => m,
        None => match provider::list_models_unified(&provider_kind, &provider_url, &cfg).await {
            Ok(v) if !v.is_empty() => v[0].id.clone(),
            _ => anyhow::bail!("No model available (is Ollama/LM Studio running?) — pass --model explicitly"),
        },
    };

    // Project context (best-effort): powers grounding + verifier_clean scoring.
    let (proj_opt, files) = match project {
        Some(p) => {
            let proj = projects::resolve_project(Some(p))?;
            let files = core_fs::list_project_files(&proj).unwrap_or_default();
            (Some(proj), files)
        }
        None => (None, Vec::new()),
    };
    let context = if let Some(proj) = proj_opt.as_ref() {
        if files.is_empty() {
            None
        } else {
        let cfg_inner = load_config().unwrap_or_default();
        let index_opt = crate::core::index::load_index(proj).ok().flatten();
        let use_hybrid = index_opt.as_ref().map(|i| !crate::core::index::needs_rebuild(proj, i)).unwrap_or(false);
        if use_hybrid {
            let emb = crate::core::embeddings::get_embedder(&cfg_inner);
            Some(intelligence::build_project_context_hybrid("benchmark", &files, index_opt.as_ref(), Some(emb), |p| {
                core_fs::read_project_file(proj, p).ok()
            }))
        } else {
            Some(intelligence::build_project_context("benchmark", &files, |p| {
                core_fs::read_project_file(proj, p).ok()
            }))
        }
        }
    } else {
        None
    };

    println!("{}", style(format!("MODEL LAB — {} ({} prompts{})", model_id, prompts.len(), suite.as_ref().map(|s| format!(", suite={}", s)).unwrap_or_default())).bold());
    let mut results = Vec::new();
    for prompt in &prompts {
        print!("  [{}] {}… ", prompt.id, prompt.suite);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let system = match &context {
            Some(c) => format!("You are Local AI, a grounded coding assistant. Use ONLY the project context. Be concise.\n\n{}", c),
            None => "You are Local AI, a concise coding assistant.".to_string(),
        };
        let messages = vec![
            provider::ChatMessage { role: "system".into(), content: system },
            provider::ChatMessage { role: "user".into(), content: prompt.user.clone() },
        ];
        // Inline streaming measurement (TTFT = first chunk, TPS ≈ chars/4/secs).
        // Kept inline (not via `core_bench::measure`) so the `Send` sink
        // required by `stream_chat_unified` doesn't hit HRTB lifetime issues.
        let start = std::time::Instant::now();
        let mut first_ms: Option<u64> = None;
        let mut streamed_chars = 0usize;
        let mut sink = |chunk: &str| {
            if first_ms.is_none() {
                first_ms = Some(start.elapsed().as_millis() as u64);
            }
            streamed_chars += chunk.len();
        };
        let output = provider::stream_chat_unified(&provider_kind, &provider_url, &cfg, &model_id, messages, 0.4, &mut sink)
            .await
            .unwrap_or_else(|e| format!("(model error: {})", e));
        let secs = start.elapsed().as_secs_f64();
        let ttft_ms = first_ms.unwrap_or((secs * 1000.0) as u64);
        let tps = core_bench::estimate_tps(streamed_chars.max(output.len()), secs);
        let verifier_ok = match (&proj_opt, prompt.kind.as_str()) {
            (Some(proj), "verifier_clean") => {
                let report = verifier::verify_response_hybrid(&output, proj, &files, "balanced", false);
                Some(!report.is_hallucinated)
            }
            _ => None,
        };
        // verifier_clean without a project fails closed with a note (see score_prompt).
        let (pass, note) = core_bench::score_prompt(prompt, &output, verifier_ok);
        println!("{} {:.1}s TTFT {}ms TPS {:.1} — {}", if pass { style("✓").green().to_string() } else { style("✗").red().to_string() }, secs, ttft_ms, tps, note);
        results.push(core_bench::PromptResult {
            id: prompt.id.clone(),
            suite: prompt.suite.clone(),
            secs,
            ttft_ms,
            tps,
            chars: output.len(),
            pass,
            note,
        });
    }

    let result = core_bench::BenchResult::summarize(&model_id, suite.as_deref().unwrap_or("all"), results);
    let dir = match out {
        Some(d) => d,
        None => core_bench::default_results_dir()?,
    };
    let path = core_bench::save_result(&dir, &result)?;
    println!(
        "\n{} {} — pass@1 {:.2} | mean TPS {:.1} | mean TTFT {}ms → {}",
        style("MODEL LAB").bold(),
        style(&model_id).cyan(),
        result.pass_at_1,
        result.mean_tps,
        result.mean_ttft_ms,
        path.display()
    );
    Ok(())
}

async fn handle_compare(a: PathBuf, b: PathBuf) -> Result<()> {
    let ra = core_bench::load_result(&a)?;
    let rb = core_bench::load_result(&b)?;
    println!("{}", style(format!("COMPARE — {} vs {}", ra.model, rb.model)).bold());
    println!("  {:<28} {:>10} {:>10} {:>8}", "prompt", ra.model.chars().take(10).collect::<String>(), rb.model.chars().take(10).collect::<String>(), "Δtps");
    for row in core_bench::compare(&ra, &rb) {
        let mark = |p: bool| if p { style("✓").green().to_string() } else { style("✗").red().to_string() };
        println!(
            "  {:<28} {:>10} {:>10} {:>+8.1}",
            row.id,
            mark(row.a_pass),
            mark(row.b_pass),
            row.b_tps - row.a_tps
        );
    }
    println!(
        "\n  pass@1: {:.2} → {:.2} | TPS: {:.1} → {:.1} | TTFT: {}ms → {}ms",
        ra.pass_at_1, rb.pass_at_1, ra.mean_tps, rb.mean_tps, ra.mean_ttft_ms, rb.mean_ttft_ms
    );
    let (ok, line) = core_bench::gate_verdict(&ra, &rb);
    println!("  {}", if ok { style(line).green() } else { style(line).red() });
    Ok(())
}
