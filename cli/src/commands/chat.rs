use anyhow::Result;
use clap::Parser;
use console::style;
use std::io::{self, Write};

use crate::core::config::{ProviderKind, load_config};
use crate::core::{fs as core_fs, intelligence, projects, provider};

#[derive(Parser)]
pub struct ChatArgs {
    /// Message to send (if empty, enters interactive REPL)
    #[arg(default_value = "")]
    pub message: String,

    /// Model ID (default: first available from active provider)
    #[arg(long)]
    pub model: Option<String>,

    /// Project id, name, or path (default: current dir)
    #[arg(long)]
    pub project: Option<String>,

    /// Disable project context injection
    #[arg(long)]
    pub no_context: bool,

    /// Do not save chat to projects.json
    #[arg(long)]
    pub no_save: bool,

    /// System prompt override
    #[arg(long)]
    pub system: Option<String>,

    /// Show project context that will be injected (debug)
    #[arg(long)]
    pub show_context: bool,

    /// Grounding mode: strict | balanced | creative
    #[arg(long)]
    pub grounding: Option<String>,

    /// Provider override for this chat (auto|lmstudio|ollama|llamacpp|generic)
    #[arg(long)]
    pub provider: Option<String>,

    /// Provider URL override
    #[arg(long)]
    pub url: Option<String>,

    /// Show verifier report (grounding check)
    #[arg(long)]
    pub show_verifier: bool,

    /// Disable verifier (not recommended, allows hallucinations)
    #[arg(long)]
    pub no_verify: bool,

    /// Verify only (run verifier on last response, no LLM call)
    #[arg(long)]
    pub verify_only: bool,

    /// Use tool calling (requires model + provider that supports tools: Ollama/LM Studio with tool-capable model)
    #[arg(long)]
    pub tools: bool,
}

fn system_prompt(project_context: &str, grounding: &str, use_tools: bool) -> String {
    let mode_prefix = if crate::core::config::is_plan_mode() {
        "PLAN MODE ACTIVE — you are in READ-ONLY planning. Do NOT output <CREATE_FILE>/<EDIT>/<DELETE>/<EXEC> blocks. Instead output a detailed PLAN listing files to create/modify, steps, risks, and alternatives. Building/writing is disabled until user switches to --mode build.\n\n"
    } else { "" };
    let header = match grounding {
        "strict" => "You are Local AI. STRICT GROUNDING — violation = failure:\n- Use ONLY files in AUTHORITATIVE CONTEXT. If answer not in context, reply: \"Not in project context — relevant file would be: <suggest path>\".\n- Every file path MUST be from inventory. Cite source [path:line].\n- Temperature low, factual only.",
        "creative" => "You are Local AI. CREATIVE MODE — you may brainstorm but prefix hypotheticals with \"Hypothetical:\". Prefer real files when possible.",
        _ => "You are Local AI, a local coding assistant. Use ONLY real project files as source of truth, do not invent. Cite file paths. Keep answers grounded.",
    };
    let tool_instruction = if use_tools {
        "\n\nTOOL USE (MANDATORY for file/web questions, you have browser + terminal access):\n- Tools: list_project_files(), read_project_file(path), search_project(query), exec(command), web_fetch(url).\n- exec: terminal commands to read all necessary files — e.g. 'ls -R', 'cat src/main.rs', 'find . -name \"*.rs\" | head -20', 'grep -rn \"TODO\"'. Read-only in plan mode.\n- web_fetch: browser access — fetch URL like 'https://doc.rust-lang.org/book/' or 'https://crates.io/crates/fastembed', returns text (4000 chars, truncated).\n- If the user asks to read, explain, or summarize a file, you MUST call read_project_file or exec before answering.\n- If you claim a file exists, you MUST have called list_project_files or read_project_file and received confirmation.\n- For web/docs questions, you MUST call web_fetch to get real content — do not hallucinate URLs.\n- Do not hallucinate file/web contents — call the tool and use its returned content.\n- Example: user asks 'read cli/src/main.rs' → call read_project_file(path=\"cli/src/main.rs\") then answer.\n- Example: 'latest Rust docs for tokio' → call web_fetch(url=\"https://docs.rs/tokio/latest/tokio/\").\n"
    } else { "" };
    let editing_rules = if crate::core::config::is_plan_mode() {
        "\n\nPLAN MODE: Do NOT output <CREATE_FILE>/<EDIT>/<DELETE>/<EXEC> blocks. Output PLAN only (steps, files, risks). User will switch to build mode to apply.\n"
    } else {
        "\n\nFILE EDITING RULES (if user asks to change files, return machine-readable blocks):\n<CREATE_FILE>FILE: path CONTENT: ... </CREATE_FILE>\n<EDIT>FILE: path SEARCH: ... REPLACE: ... </EDIT>\n<DELETE>FILE: path </DELETE>\n<EXEC>COMMAND: ... </EXEC>\nRules: use relative paths, SEARCH must match exactly.\n"
    };
    format!(
        r#"{mode_prefix}{header}{tool_instruction}

IMPORTANT PROJECT RULES:
- Real project files are source of truth, do not invent.
- Analyze actual contents, mention file paths.
- Keep answers grounded in provided context.
{editing_rules}
PROJECT CONTEXT:
{project_context}"#
    )
}

fn grounding_temperature(grounding: &str) -> f32 {
    match grounding {
        "strict" => 0.2,
        "creative" => 0.7,
        _ => 0.4, // balanced default
    }
}

async fn resolve_model(
    requested: Option<String>,
    provider_kind: &ProviderKind,
    provider_url: &str,
    cfg: &crate::core::config::AppConfig,
) -> Result<String> {
    if let Some(m) = requested { return Ok(m); }
    // Try unified list
    let url = if provider_url.is_empty() { "" } else { provider_url };
    let kind = if provider_url.is_empty() && provider_kind == &ProviderKind::Auto { &ProviderKind::Auto } else { provider_kind };
    match provider::list_models_unified(kind, url, cfg).await {
        Ok(models) if !models.is_empty() => Ok(models[0].id.clone()),
        _ => Ok("qwen/qwen3.5-9b".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn chat_once(
    model: &str,
    project: &projects::Project,
    user_text: &str,
    no_context: bool,
    show_context: bool,
    show_verifier: bool,
    no_verify: bool,
    use_tools: bool,
    save: bool,
    system_override: Option<String>,
    grounding: &str,
    provider_kind: ProviderKind,
    provider_url: String,
) -> Result<()> {
    let cfg = load_config().unwrap_or_default();
    let messages_hist = project.messages.clone();
    let files = if !no_context {
        core_fs::list_project_files(project).unwrap_or_default()
    } else { vec![] };

    // Layer 0: deterministic router — no LLM for inventory/project-name
    if !no_context && !files.is_empty() {
        if crate::core::verifier::is_project_name_query(user_text) {
            let resp = format!("The current project is **{}** (path: {})", project.name, project.folder_path.as_deref().unwrap_or("-"));
            println!("{} {}", style("Assistant:").cyan().bold(), resp);
            // Fall through to save below, don't call LLM
            if save {
                let mut updated = project.clone();
                updated.messages.push(projects::ChatMessage { role: "user".into(), content: user_text.to_string() });
                updated.messages.push(projects::ChatMessage { role: "assistant".into(), content: resp.clone() });
                updated.updated_at = chrono::Utc::now().to_rfc3339();
                if projects::find_project(&project.id)?.is_some() { projects::save_project(updated)?; }
                else if project.folder_path.is_some() { let mut ps = projects::read_projects()?; ps.push(updated); projects::write_projects(&ps)?; }
            }
            return Ok(());
        }
        if crate::core::verifier::is_inventory_query(user_text) {
            let resp = crate::core::verifier::deterministic_inventory_response(project, &files);
            println!("{} {}", style("Assistant:").cyan().bold(), resp);
            if show_verifier { eprintln!("{} deterministic inventory (no LLM), {} files", style("[verifier]").dim(), files.len()); }
            if save {
                let mut updated = project.clone();
                updated.messages.push(projects::ChatMessage { role: "user".into(), content: user_text.to_string() });
                updated.messages.push(projects::ChatMessage { role: "assistant".into(), content: resp.clone() });
                updated.updated_at = chrono::Utc::now().to_rfc3339();
                if projects::find_project(&project.id)?.is_some() { projects::save_project(updated)?; }
                else if project.folder_path.is_some() { let mut ps = projects::read_projects()?; ps.push(updated); projects::write_projects(&ps)?; }
            }
            return Ok(());
        }
    }

    let project_context = if no_context || files.is_empty() {
        String::new()
    } else {
        // Try hybrid retrieval if index exists and is fresh
        let cfg_inner = load_config().unwrap_or_default();
        let index_opt = crate::core::index::load_index(project).ok().flatten();
        let use_hybrid = if let Some(idx) = &index_opt {
            !crate::core::index::needs_rebuild(project, idx)
        } else { false };
        if use_hybrid {
            let embedder = crate::core::embeddings::get_embedder(&cfg_inner);
            crate::core::intelligence::build_project_context_hybrid(
                user_text, &files, index_opt.as_ref(), Some(embedder),
                |p| core_fs::read_project_file(project, p).ok()
            )
        } else {
            // Fallback: keyword + inventory header (still grounded vs old version)
            intelligence::build_project_context(user_text, &files, |p| {
                core_fs::read_project_file(project, p).ok()
            })
        }
    };

    if show_context && !project_context.is_empty() {
        eprintln!("{}--- PROJECT CONTEXT ({} chars, grounding={}) ---\n{}\n--- END CONTEXT ---",
            style("[debug] ").dim(), project_context.len(), grounding,
            &project_context[..project_context.len().min(2000)]);
    }

    let grounding_mode = grounding.to_string();
    let system_content = system_override.clone().unwrap_or_else(|| system_prompt(&project_context, &grounding_mode, use_tools));
    let mut lm_messages = vec![provider::ChatMessage { role: "system".into(), content: system_content }];
    let recent: Vec<_> = messages_hist.iter().rev().take(2).collect();
    let mut recent_rev: Vec<_> = recent.into_iter().rev().cloned().collect();
    for m in recent_rev.drain(..) {
        lm_messages.push(provider::ChatMessage { role: m.role, content: m.content });
    }
    lm_messages.push(provider::ChatMessage { role: "user".into(), content: user_text.to_string() });

    // Layer 3: tool calling (optional, --tools)
    if use_tools && !no_context {
        let provider_supports = {
            let p = provider::get_provider(&provider_kind);
            p.supports_tools() || provider_kind == ProviderKind::Auto
        };
        let model_supports = crate::core::tools::supports_tools_for_model(model);
        if provider_supports && model_supports {
            if show_verifier { eprintln!("{} tool calling enabled (model: {}, provider: {})", style("[tools]").dim(), model, provider_kind); }
            let tools = crate::core::tools::project_tools();
            match provider::chat_with_tools_unified(&provider_kind, &provider_url, &cfg, model, lm_messages.clone(), tools.clone(), grounding_temperature(grounding)).await {
                Ok((tool_content, tool_calls)) if !tool_calls.is_empty() => {
                    eprintln!("{} model requested {} tool calls", style("[tools]").dim(), tool_calls.len());
                    let mut tool_results_text = String::new();
                    for call in &tool_calls {
                        match crate::core::tools::execute_tool(project, call).await {
                            Ok(val) => {
                                let pretty = serde_json::to_string_pretty(&val).unwrap_or(val.to_string());
                                eprintln!("  {} {} -> {}", style("→").cyan(), call.name, pretty.lines().next().unwrap_or(""));
                                tool_results_text.push_str(&format!("Tool '{}' with args {} returned:\n{}\n\n", call.name, call.arguments, pretty));
                            }
                            Err(e) => {
                                eprintln!("  {} {} error: {}", style("✗").red(), call.name, e);
                                tool_results_text.push_str(&format!("Tool '{}' error: {}\n", call.name, e));
                            }
                        }
                    }
                    // Final streaming with tool results
                    let mut with_tools = lm_messages.clone();
                    with_tools.push(provider::ChatMessage { role: "assistant".into(), content: format!("Tool calls: {}", tool_calls.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(", ")) });
                    with_tools.push(provider::ChatMessage { role: "user".into(), content: format!("Tool results (authoritative, do not hallucinate):\n{}\nAnswer the original question '{}' using only these verified results. Cite files.", tool_results_text, user_text) });
                    print!("{} ", style("Assistant (tools):").cyan().bold());
                    io::stdout().flush()?;
                    let mut full2 = String::new();
                    let resp2 = provider::stream_chat_unified(&provider_kind, &provider_url, &cfg, model, with_tools, grounding_temperature(grounding), &mut |chunk| {
                        print!("{}", chunk);
                        let _ = io::stdout().flush();
                        full2.push_str(chunk);
                    }).await;
                    let mut assistant_content = match resp2 {
                        Ok(s) if !s.is_empty() => s,
                        Ok(_) => full2.clone(),
                        Err(e) => { eprintln!("\nTool final chat failed: {}", e); full2.clone() }
                    };
                    println!("\n");
                    // Verifier for tool final — hybrid lexical + Python (groundrails/LettuceDetect) when --show-verifier
                    let verifier_report = if !no_verify && !files.is_empty() {
                        let report = crate::core::verifier::verify_response_hybrid(&assistant_content, project, &files, grounding, show_verifier);
                        if show_verifier {
                            eprintln!("{} verifier (tools): {} mentioned, {} invented, score {:.2}, hallucinated={} (mode={})", style("[verifier]").dim(), report.total_mentioned, report.invented_count, report.hallucination_score, report.is_hallucinated, report.grounding_mode);
                            if !report.invented_files.is_empty() { eprintln!("  {} invented: {}", style("✗").red(), report.invented_files.join(", ")); }
                            if !report.edit_block_errors.is_empty() { for e in &report.edit_block_errors { eprintln!("  {} edit: {}", style("✗").red(), e); } }
                            // ragground citation preview when requested
                            let cited = crate::core::verifier::inject_citations(&assistant_content, &report);
                            if cited != assistant_content { eprintln!("  {} cited preview: {}", style("[cited]").dim(), cited.lines().next().unwrap_or("")) }
                        }
                        Some(report)
                    } else { None };
                    let edits = parse_edits(&assistant_content);
                    if !edits.is_empty() {
                        if crate::core::config::is_plan_mode() {
                            println!("{}", style(format!("Plan mode — AI proposed {} file operation(s) (NOT applied, read-only):", edits.len())).yellow());
                            for (i, e) in edits.iter().enumerate() { println!("  {}. {} — {}", i+1, e.kind, e.path); }
                            println!("  {} Plan mode — switch to build: --mode build", style("→").dim());
                        } else {
                            println!("{}", style(format!("AI proposed {} file operation(s):", edits.len())).yellow());
                            for (i, e) in edits.iter().enumerate() { println!("  {}. {} — {}", i+1, e.kind, e.path); }
                        }
                    }
                    if save {
                        let mut updated = project.clone();
                        updated.messages.push(projects::ChatMessage { role: "user".into(), content: user_text.to_string() });
                        updated.messages.push(projects::ChatMessage { role: "assistant".into(), content: assistant_content.clone() });
                        updated.updated_at = chrono::Utc::now().to_rfc3339();
                        if projects::find_project(&project.id)?.is_some() { projects::save_project(updated)?; }
                        else if project.folder_path.is_some() { let mut ps = projects::read_projects()?; ps.push(updated); projects::write_projects(&ps)?; }
                    }
                    return Ok(());
                }
                Ok((tool_content, _)) if !tool_content.is_empty() => {
                    if show_verifier { eprintln!("{} tools: no tool calls, using direct content ({} chars)", style("[tools]").dim(), tool_content.len()); }
                    // Treat tool_content as assistant content and continue to verifier (skip streaming)
                    let mut assistant_content = tool_content;
                    println!("{} {}", style("Assistant:").cyan().bold(), assistant_content);
                    println!();
                    let verifier_report = if !no_verify && !files.is_empty() {
                        let report = crate::core::verifier::verify_response_hybrid(&assistant_content, project, &files, grounding, show_verifier);
                        if show_verifier {
                            eprintln!("{} verifier: {} mentioned, {} invented, score {:.2}, hallucinated={}", style("[verifier]").dim(), report.total_mentioned, report.invented_count, report.hallucination_score, report.is_hallucinated);
                            if !report.invented_files.is_empty() { eprintln!("  {} invented: {}", style("✗").red(), report.invented_files.join(", ")); }
                            let cited = crate::core::verifier::inject_citations(&assistant_content, &report);
                            if cited != assistant_content { eprintln!("  {} cited: {}", style("[cited]").dim(), cited.lines().next().unwrap_or("")) }
                        }
                        Some(report)
                    } else { None };
                    let edits = parse_edits(&assistant_content);
                    if !edits.is_empty() {
                        if crate::core::config::is_plan_mode() {
                            println!("{}", style(format!("Plan mode — AI proposed {} file operation(s) (NOT applied, read-only):", edits.len())).yellow());
                            for (i, e) in edits.iter().enumerate() { println!("  {}. {} — {}", i+1, e.kind, e.path); }
                            println!("  {} Plan mode — switch to build: --mode build", style("→").dim());
                        } else {
                            println!("{}", style(format!("AI proposed {} file operation(s):", edits.len())).yellow());
                            for (i, e) in edits.iter().enumerate() { println!("  {}. {} — {}", i+1, e.kind, e.path); }
                        }
                    }
                    if save {
                        let mut updated = project.clone();
                        updated.messages.push(projects::ChatMessage { role: "user".into(), content: user_text.to_string() });
                        updated.messages.push(projects::ChatMessage { role: "assistant".into(), content: assistant_content.clone() });
                        updated.updated_at = chrono::Utc::now().to_rfc3339();
                        if projects::find_project(&project.id)?.is_some() { projects::save_project(updated)?; }
                        else if project.folder_path.is_some() { let mut ps = projects::read_projects()?; ps.push(updated); projects::write_projects(&ps)?; }
                    }
                    return Ok(());
                }
                Ok(_) => {
                    if show_verifier { eprintln!("{} tools: no calls, falling back to normal chat", style("[tools]").dim()); }
                }
                Err(e) => {
                    if show_verifier { eprintln!("{} tool chat failed ({}), falling back to normal", style("[tools]").dim(), e); }
                }
            }
        } else {
            if show_verifier && use_tools {
                if !provider_supports { eprintln!("{} tools not supported for provider {} (use Ollama/LM Studio with tool-capable model)", style("[tools]").dim(), provider_kind); }
                if !model_supports { eprintln!("{} model {} may not support tools (0.5b too small), using normal chat", style("[tools]").dim(), model); }
            }
        }
    }

    print!("{} ", style("Assistant:").cyan().bold());
    io::stdout().flush()?;
    let mut full = String::new();
    let temperature = grounding_temperature(grounding);
    let resp = provider::stream_chat_unified(
        &provider_kind,
        &provider_url,
        &cfg,
        model,
        lm_messages,
        temperature,
        &mut |chunk| {
            print!("{}", chunk);
            let _ = io::stdout().flush();
            full.push_str(chunk);
        },
    ).await;

    let mut assistant_content = match resp {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => full.clone(),
        Err(e) => {
            eprintln!("\n{} {} (provider={}, url={})", style("Error:").red(), e, provider_kind, if provider_url.is_empty() { "auto".into() } else { provider_url.clone() });
            eprintln!("  Try: local-ai doctor  or  local-ai models list");
            return Ok(());
        }
    };
    println!("\n");

    // Layer 4: verifier (post-generation) — hybrid lexical + optional Python external (groundrails/LettuceDetect/ragground)
    let mut verifier_report = if !no_verify && !files.is_empty() {
        let report = crate::core::verifier::verify_response_hybrid(&assistant_content, project, &files, grounding, show_verifier);
        if show_verifier {
            eprintln!("{} verifier: {} mentioned, {} invented, score {:.2}, hallucinated={} (mode={})",
                style("[verifier]").dim(), report.total_mentioned, report.invented_count, report.hallucination_score, report.is_hallucinated, report.grounding_mode);
            if !report.invented_files.is_empty() {
                eprintln!("  {} invented: {}", style("✗").red(), report.invented_files.join(", "));
            }
            if !report.edit_block_errors.is_empty() {
                for err in &report.edit_block_errors {
                    eprintln!("  {} edit: {}", style("✗").red(), err);
                }
            }
            for sc in &report.symbol_checks {
                if !sc.found {
                    eprintln!("  {} symbol not found: {} ( hallucinated )", style("?").yellow(), sc.symbol);
                }
            }
        }
        // Auto-retry once if strict and hallucinated
        if report.is_hallucinated && grounding == "strict" && !no_context {
            eprintln!("{} strict grounding detected hallucination ({} invented), retrying with temp 0.2 + stronger inventory...", style("⚠").yellow(), report.invented_count);
            // Build retry messages with stronger instruction
            let retry_system = format!("{}\n\nPREVIOUS ANSWER HALLUCINATED invented files: {}. You MUST only use files from AUTHORITATIVE inventory. If not in context, say \"Not in project context\".", 
                if let Some(s) = system_override.clone() { s } else { system_prompt(&project_context, grounding, use_tools) },
                report.invented_files.join(", ")
            );
            let mut retry_messages = vec![provider::ChatMessage { role: "system".into(), content: retry_system }];
            for m in messages_hist.iter().rev().take(2).rev() {
                retry_messages.push(provider::ChatMessage { role: m.role.clone(), content: m.content.clone() });
            }
            retry_messages.push(provider::ChatMessage { role: "user".into(), content: user_text.to_string() });
            let mut retry_full = String::new();
            print!("{} ", style("Assistant (retry):").cyan().bold());
            io::stdout().flush()?;
            let retry_resp = provider::stream_chat_unified(
                &provider_kind, &provider_url, &cfg, model, retry_messages, 0.2,
                &mut |chunk| { print!("{}", chunk); let _ = io::stdout().flush(); retry_full.push_str(chunk); }
            ).await;
            let retry_content = match retry_resp {
                Ok(s) if !s.is_empty() => s,
                Ok(_) => retry_full.clone(),
                Err(e) => { eprintln!("\nRetry failed: {}", e); assistant_content.clone() }
            };
            println!("\n");
            if !retry_content.is_empty() {
                // Re-verify retry — hybrid
                let retry_report = crate::core::verifier::verify_response_hybrid(&retry_content, project, &files, grounding, show_verifier);
                if show_verifier {
                    eprintln!("{} retry verifier: {} invented, score {:.2}, hallucinated={}", style("[verifier]").dim(), retry_report.invented_count, retry_report.hallucination_score, retry_report.is_hallucinated);
                }
                if !retry_report.is_hallucinated {
                    eprintln!("{} retry succeeded (no hallucination), using retried answer", style("✓").green());
                    assistant_content = retry_content;
                } else {
                    eprintln!("{} retry still hallucinated ({} invented, was {}), returning grounded fallback", style("✗").red(), retry_report.invented_count, report.invented_count);
                    let closest: Vec<String> = files.iter().filter(|f| !f.is_directory).take(5).map(|f| f.path.clone()).collect();
                    let fallback = format!(
                        "Not in project context — file `{}` not found in attached project ({} files). The model hallucinated invented files: {}. Closest real files: {}. Use `local-ai files list --project {}` to see inventory. If you want to create this file, ask with `<CREATE_FILE>`.",
                        report.invented_files.first().unwrap_or(&user_text.to_string()),
                        files.len(),
                        report.invented_files.join(", "),
                        closest.join(", "),
                        project.name
                    );
                    println!("{} {}", style("Assistant (fallback):").cyan().bold(), fallback);
                    assistant_content = fallback;
                }
            }
        } else if report.is_hallucinated {
            eprintln!("{} hallucination detected (score {:.2}) — invented: {}. In strict mode this would auto-retry.", style("⚠").yellow(), report.hallucination_score, report.invented_files.join(", "));
        } else if report.total_mentioned > 0 && show_verifier {
            eprintln!("{} all {} files verified", style("✓").green(), report.total_mentioned);
        }
        Some(report)
    } else { None };

    // Also show verifier for edit block errors even if no file mentions
    if let Some(ref rep) = verifier_report {
        if !rep.edit_block_errors.is_empty() && !show_verifier {
            for err in &rep.edit_block_errors {
                eprintln!("{} {}", style("edit verifier:").yellow(), err);
            }
        }
    }

    let edits = parse_edits(&assistant_content);
    if !edits.is_empty() {
        if crate::core::config::is_plan_mode() {
            println!("{}", style(format!("Plan mode — AI proposed {} file operation(s) (NOT applied, read-only):", edits.len())).yellow());
            for (i, e) in edits.iter().enumerate() {
                println!("  {}. {} — {}", i+1, e.kind, e.path);
            }
            println!("  {} Plan mode is active. To apply, switch to build mode: --mode build", style("→").dim());
        } else {
            println!("{}", style(format!("AI proposed {} file operation(s):", edits.len())).yellow());
            for (i, e) in edits.iter().enumerate() {
                println!("  {}. {} — {}", i+1, e.kind, e.path);
            }
            println!("  Use `local-ai files` or approve via chat integration to apply.");
        }
    }

    if save {
        let mut updated = project.clone();
        updated.messages.push(projects::ChatMessage { role: "user".into(), content: user_text.to_string() });
        updated.messages.push(projects::ChatMessage { role: "assistant".into(), content: assistant_content });
        updated.updated_at = chrono::Utc::now().to_rfc3339();
        if projects::find_project(&project.id)?.is_some() {
            projects::save_project(updated)?;
        } else if project.folder_path.is_some() {
            let mut ps = projects::read_projects()?;
            ps.push(updated);
            projects::write_projects(&ps)?;
        }
    }
    Ok(())
}

struct EditSummary { kind: String, path: String }
fn parse_edits(s: &str) -> Vec<EditSummary> {
    let mut out = Vec::new();
    let mut idx = 0;
    while let Some(pos) = s[idx..].find("<CREATE_FILE>") {
        let start = idx + pos;
        if let Some(end) = s[start..].find("</CREATE_FILE>") {
            let block = &s[start..start+end];
            let path = block.split("FILE:").nth(1).unwrap_or("").split("CONTENT:").next().unwrap_or("").trim().to_string();
            out.push(EditSummary{ kind: "CREATE".into(), path });
            idx = start + end + 14;
        } else { break; }
    }
    idx = 0;
    while let Some(pos) = s[idx..].find("<EDIT>") {
        let start = idx + pos;
        if let Some(end) = s[start..].find("</EDIT>") {
            let block = &s[start..start+end];
            if block.contains("FILE:") {
                let path = block.split("FILE:").nth(1).unwrap_or("").split("SEARCH:").next().unwrap_or("").split("ACTION:").next().unwrap_or("").trim().to_string();
                out.push(EditSummary{ kind: "EDIT/DELETE".into(), path });
            }
            idx = start + end + 7;
        } else { break; }
    }
    idx = 0;
    while let Some(pos) = s[idx..].find("<DELETE>") {
        let start = idx + pos;
        if let Some(end) = s[start..].find("</DELETE>") {
            let block = &s[start..start+end];
            let path = block.split("FILE:").nth(1).unwrap_or("").trim().to_string();
            out.push(EditSummary{ kind: "DELETE".into(), path });
            idx = start + end + 9;
        } else { break; }
    }
    idx = 0;
    while let Some(pos) = s[idx..].find("<EXEC>") {
        let start = idx + pos;
        if let Some(end) = s[start..].find("</EXEC>") {
            let block = &s[start..start+end];
            let cmd = block.split("COMMAND:").nth(1).unwrap_or("").trim().to_string();
            out.push(EditSummary{ kind: "EXEC".into(), path: cmd });
            idx = start + end + 7;
        } else { break; }
    }
    out
}

pub async fn handle(
    args: ChatArgs,
    global_provider: Option<ProviderKind>,
    global_url: Option<String>,
    global_lm_url: Option<String>,
) -> Result<()> {
    let cfg = load_config().unwrap_or_default();

    // Resolve provider for this call: --provider > global --provider > config
    let provider_kind = if let Some(p) = &args.provider {
        p.parse().unwrap_or_else(|e| { eprintln!("{}", e); std::process::exit(1) })
    } else { global_provider.clone().unwrap_or_else(|| cfg.provider.active.clone()) };

    let provider_url = if let Some(u) = &args.url {
        u.clone()
    } else {
        crate::core::config::resolve_provider_url(&provider_kind, &cfg, global_url.as_deref(), global_lm_url.as_deref())
    };

    let grounding = args.grounding.clone()
        .or_else(|| std::env::var("LOCAL_AI_GROUNDING").ok())
        .unwrap_or_else(|| cfg.grounding.mode.clone());

    let project = projects::resolve_project(args.project.clone())?;
    let model = resolve_model(args.model.clone(), &provider_kind, &provider_url, &cfg).await?;

    if !args.message.trim().is_empty() {
        chat_once(&model, &project, &args.message, args.no_context, args.show_context, args.show_verifier, args.no_verify, args.tools, !args.no_save, args.system.clone(), &grounding, provider_kind, provider_url).await?;
        return Ok(());
    }

    // Interactive REPL
    println!("{} Local AI chat — model: {} — provider: {} — project: {} — type /exit to quit, /help for commands",
        style("◈").magenta(),
        style(&model).cyan(),
        style(format!("{} @ {}", provider_kind, if provider_url.is_empty() { "auto".into() } else { provider_url.clone() })).dim(),
        style(project.folder_path.as_deref().unwrap_or("no folder")).dim()
    );
    println!("  Grounding: {} (temp {:.1}) — URL: {}", grounding, grounding_temperature(&grounding), if provider_url.is_empty() { "auto".into() } else { provider_url.clone() });
    let mut hist = project.clone();
    if let Some(stored) = projects::find_project(&project.id)? {
        hist = stored;
    } else if let Some(fp) = &project.folder_path {
        if let Some(stored) = projects::find_project(fp)? { hist = stored; }
    }
    if !hist.messages.is_empty() {
        println!("  loaded {} previous messages", hist.messages.len());
    }

    let stdin = io::stdin();
    loop {
        print!("{} ", style("You:").green().bold());
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 { break; }
        let text = line.trim();
        if text.is_empty() { continue; }
        if text == "/exit" || text == "/quit" { break; }
        if text == "/help" {
            println!("  /exit, /quit — leave chat\n  /clear — clear history\n  /history — show history\n  /model <id> — switch model\n  /grounding strict|balanced|creative — switch grounding\n  /context — toggle context (currently {})", if args.no_context { "off" } else { "on" });
            continue;
        }
        if text == "/history" {
            for m in &hist.messages {
                println!("{}: {}", m.role, m.content.lines().next().unwrap_or("").chars().take(80).collect::<String>());
            }
            continue;
        }
        if text == "/clear" {
            hist.messages.clear();
            println!("history cleared");
            continue;
        }
        if text.starts_with("/model ") {
            let new_model = text[7..].trim().to_string();
            println!("model -> {}", new_model);
            chat_once(&new_model, &hist, "", args.no_context, args.show_context, args.show_verifier, args.no_verify, args.tools, !args.no_save, args.system.clone(), &grounding, provider_kind.clone(), provider_url.clone()).await?;
            continue;
        }
        if text.starts_with("/grounding ") {
            let ng = text[11..].trim().to_string();
            println!("grounding -> {} (restart chat to apply)", ng);
            continue;
        }
        let current = if let Some(stored) = projects::find_project(&hist.id)? { stored } else { hist.clone() };
        chat_once(&model, &current, text, args.no_context, args.show_context, args.show_verifier, args.no_verify, args.tools, !args.no_save, args.system.clone(), &grounding, provider_kind.clone(), provider_url.clone()).await?;
        if let Some(updated) = projects::find_project(&hist.id)? { hist = updated; }
        else if let Some(fp) = &hist.folder_path { if let Some(updated) = projects::find_project(fp)? { hist = updated; } }
    }
    println!("bye");
    Ok(())
}
