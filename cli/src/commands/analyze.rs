use anyhow::Result;
use clap::Parser;

use crate::core::config::{ProviderKind, load_config};
use crate::core::{fs as core_fs, intelligence, projects, provider};

#[derive(Parser)]
pub struct AnalyzeArgs {
    /// What to analyze: "architecture", "explain auth flow", "find database code", etc.
    #[arg(default_value = "architecture")]
    pub query: String,

    #[arg(long)]
    pub project: Option<String>,

    #[arg(long)]
    pub model: Option<String>,

    /// Show relevant files without calling LLM
    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub provider: Option<String>,

    #[arg(long)]
    pub url: Option<String>,

    #[arg(long)]
    pub grounding: Option<String>,

    #[arg(long)]
    pub show_verifier: bool,

    #[arg(long)]
    pub no_verify: bool,
}

pub async fn handle(
    args: AnalyzeArgs,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    let cfg = load_config().unwrap_or_default();
    let provider_kind = if let Some(p) = &args.provider {
        p.parse().unwrap_or_else(|e| { eprintln!("{}", e); std::process::exit(1) })
    } else { global_provider.clone().unwrap_or_else(|| cfg.provider.active.clone()) };
    let provider_url = if let Some(u) = &args.url { u.clone() } else {
        crate::core::config::resolve_provider_url(&provider_kind, &cfg, global_url.as_deref(), global_lm_url.as_deref())
    };
    let grounding = args.grounding.clone()
        .or_else(|| std::env::var("LOCAL_AI_GROUNDING").ok())
        .unwrap_or_else(|| cfg.grounding.mode.clone());

    let proj = projects::resolve_project(args.project)?;
    let files = core_fs::list_project_files(&proj)?;
    let intent = intelligence::detect_intent(&args.query);
    println!("intent: {} (confidence {:.2}) keywords: {:?}  grounding: {}", intent.intent, intent.confidence, intent.keywords, grounding);
    // Try hybrid if index exists
    let index_opt = crate::core::index::load_index(&proj).ok().flatten();
    let use_hybrid = if let Some(idx) = &index_opt { !crate::core::index::needs_rebuild(&proj, idx) } else { false };
    let relevant = if use_hybrid {
        let embedder = crate::core::embeddings::get_embedder(&cfg);
        intelligence::hybrid_rank(&args.query, &files, index_opt.as_ref(), Some(embedder))
    } else {
        intelligence::rank_relevant_files(&args.query, &files, &intent)
    };
    let top: Vec<_> = relevant.into_iter().take(8).collect();
    if top.is_empty() {
        println!("No relevant files found (project has {} files)", files.len());
        if args.dry_run {
            // Verifier dry-run preview — show what would be rejected if query mentions invented files
            if !args.no_verify && !files.is_empty() {
                let preview = crate::core::verifier::verify_response_hybrid(&args.query, &proj, &files, &grounding, args.show_verifier);
                eprintln!("\n{} verifier dry-run: {} mentioned, {} invented, score {:.2}, hallucinated={} (mode={})",
                    console::style("[verifier]").dim(), preview.total_mentioned, preview.invented_count, preview.hallucination_score, preview.is_hallucinated, preview.grounding_mode);
                if !preview.invented_files.is_empty() {
                    eprintln!("  {} would-reject invented: {}", console::style("✗").red(), preview.invented_files.join(", "));
                    eprintln!("  {} in --dry-run, no LLM call — real answer verifier runs post-generation; in strict mode exit 2 on hallucination", console::style("→").dim());
                } else if preview.total_mentioned > 0 {
                    eprintln!("  {} query mentions verified files", console::style("✓").green());
                } else {
                    eprintln!("  {} no file mentions in query — verifier will check answer post-generation", console::style("[verifier]").dim());
                }
                if !preview.edit_block_errors.is_empty() {
                    for err in &preview.edit_block_errors { eprintln!("  {} edit would-reject: {}", console::style("✗").red(), err); }
                }
                if !preview.symbol_checks.is_empty() && preview.symbol_checks.iter().any(|s| !s.found) {
                    for sc in preview.symbol_checks.iter().filter(|s| !s.found) {
                        eprintln!("  {} symbol not found would-reject: {} (hallucinated)", console::style("?").yellow(), sc.symbol);
                    }
                }
                if args.show_verifier {
                    // Show citation injection preview (ragground-style)
                    let cited = crate::core::verifier::inject_citations(&args.query, &preview);
                    if cited != args.query {
                        eprintln!("  {} cited preview (ragground): {}", console::style("[cited]").dim(), cited.lines().next().unwrap_or(""));
                    }
                    eprintln!("  {} backend: lexical{} | for full ML (groundrails+LettuceDetect) run finetune/verify.py --show-support",
                        console::style("[verifier]").dim(),
                        if std::path::Path::new("finetune/verify.py").exists() || std::path::Path::new("../finetune/verify.py").exists() { "+python (if --show-verifier)" } else { " (python finetune/verify.py not found, using Rust lexical)" });
                }
            }
            return Ok(());
        }
    } else {
        let mode = if use_hybrid { "hybrid (embeddings+keyword)" } else { "keyword" };
        println!("Top relevant files [{}]:", mode);
        for r in &top {
            println!("  {:.2}  {}  — {}", r.score, r.file.path, r.reasons.join(", "));
        }
        if use_hybrid { println!("  (used embedding index: {})", index_opt.as_ref().map(|i| i.embedder_name.as_str()).unwrap_or("?")); }
        else { println!("  (tip: run `local-ai index rebuild --project {}` for hybrid retrieval)", proj.folder_path.as_deref().unwrap_or(&proj.id)); }
    }
    if args.dry_run {
        // Verifier preview for non-empty top as well
        if !args.no_verify && !files.is_empty() {
            let preview = crate::core::verifier::verify_response_hybrid(&args.query, &proj, &files, &grounding, args.show_verifier);
            eprintln!("\n{} verifier dry-run: {} mentioned, {} invented, score {:.2}, hallucinated={} (mode={})",
                console::style("[verifier]").dim(), preview.total_mentioned, preview.invented_count, preview.hallucination_score, preview.is_hallucinated, preview.grounding_mode);
            if !preview.invented_files.is_empty() {
                eprintln!("  {} would-reject invented: {}", console::style("✗").red(), preview.invented_files.join(", "));
                eprintln!("  {} real answer verifier runs post-generation; strict mode would exit 2", console::style("→").dim());
            } else if preview.total_mentioned > 0 {
                eprintln!("  {} query mentions verified", console::style("✓").green());
            } else {
                eprintln!("  {} no file mentions in query — verifier will check answer post-generation", console::style("[verifier]").dim());
            }
            if !preview.edit_block_errors.is_empty() {
                for err in &preview.edit_block_errors { eprintln!("  {} edit would-reject: {}", console::style("✗").red(), err); }
            }
            if args.show_verifier {
                let cited = crate::core::verifier::inject_citations(&args.query, &preview);
                if cited != args.query {
                    eprintln!("  {} cited preview: {}", console::style("[cited]").dim(), cited.lines().next().unwrap_or(""));
                }
                eprintln!("  {} backend: lexical{}",
                    console::style("[verifier]").dim(),
                    if std::path::Path::new("finetune/verify.py").exists() || std::path::Path::new("../finetune/verify.py").exists() { "+python" } else { "" });
            }
        }
        return Ok(());
    }

    let model = if let Some(m) = args.model.clone() {
        m
    } else {
        let kind = provider_kind.clone();
        let url = provider_url.clone();
        match provider::list_models_unified(&kind, &url, &cfg).await {
            Ok(v) if !v.is_empty() => v[0].id.clone(),
            _ => "qwen/qwen3.5-9b".to_string(),
        }
    };
    let context = {
        let idx = crate::core::index::load_index(&proj).ok().flatten();
        let use_hybrid = if let Some(i) = &idx { !crate::core::index::needs_rebuild(&proj, i) } else { false };
        if use_hybrid {
            let emb = crate::core::embeddings::get_embedder(&cfg);
            intelligence::build_project_context_hybrid(&args.query, &files, idx.as_ref(), Some(emb), |p| core_fs::read_project_file(&proj, p).ok())
        } else {
            intelligence::build_project_context(&args.query, &files, |p| core_fs::read_project_file(&proj, p).ok())
        }
    };
    let temp = match grounding.as_str() {
        "strict" => 0.2, "creative" => 0.7, _ => 0.4,
    };
    let system = format!("You are Local AI. Analyze the project. Be concise, cite file paths, be factual. Grounding: {}.\n\n{}", grounding, context);
    let messages = vec![
        provider::ChatMessage { role: "system".into(), content: system },
        provider::ChatMessage { role: "user".into(), content: args.query.clone() },
    ];
    println!("\n--- Analysis (model: {} provider: {} temp: {}) ---\n", model, provider_kind, temp);
    let full = provider::stream_chat_unified(&provider_kind, &provider_url, &cfg, &model, messages, temp, &mut |chunk| {
        print!("{}", chunk);
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }).await?;
    if full.is_empty() {
        eprintln!("(no response — is {} running? Try local-ai doctor)", provider_kind);
    } else {
        println!("\n");
        // Verifier for analyze (post-generation) — hybrid lexical + optional Python (groundrails/LettuceDetect)
        if !args.no_verify && !files.is_empty() {
            let report = crate::core::verifier::verify_response_hybrid(&full, &proj, &files, &grounding, args.show_verifier);
            if args.show_verifier || report.is_hallucinated {
                eprintln!("{} verifier: {} mentioned, {} invented, score {:.2}, hallucinated={} (mode={})",
                    console::style("[verifier]").dim(), report.total_mentioned, report.invented_count, report.hallucination_score, report.is_hallucinated, report.grounding_mode);
                if !report.invented_files.is_empty() {
                    eprintln!("  {} invented: {}", console::style("✗").red(), report.invented_files.join(", "));
                }
                if report.is_hallucinated {
                    eprintln!("  {} hallucination detected — in strict mode would retry or show `Not in context`", console::style("⚠").yellow());
                } else if report.total_mentioned > 0 {
                    eprintln!("  {} all files verified", console::style("✓").green());
                }
            }
            // For CI: exit 2 if hallucinated and strict
            if report.is_hallucinated && grounding == "strict" {
                std::process::exit(2);
            }
        }
    }
    Ok(())
}
