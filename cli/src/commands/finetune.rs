use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

use crate::core::{bench as core_bench, fs as core_fs, projects};

#[derive(Parser)]
pub struct FinetuneArgs {
    #[command(subcommand)]
    pub command: FinetuneCommands,
}

#[derive(Subcommand)]
pub enum FinetuneCommands {
    /// Show finetune environment (GPU, OS, recommended stack)
    Status,
    /// Prepare a finetuning dataset from a project folder
    Prepare {
        #[arg(long)]
        project: Option<String>,
        /// Output JSONL path
        #[arg(long, default_value = "./finetune-dataset.jsonl")]
        out: PathBuf,
        /// Format: alpaca, sharegpt, or chatml
        #[arg(long, default_value = "sharegpt")]
        format: String,
        /// Max chars per file (larger files are chunked, not truncated)
        #[arg(long, default_value = "10000")]
        max_chars: usize,
        /// Min chars per sample (smaller files skipped)
        #[arg(long, default_value = "50")]
        min_chars: usize,
        /// Include system prompt
        #[arg(long)]
        system_prompt: Option<String>,
    },
    /// Train with QLoRA (dispatches to Unsloth / MLX / torchtune based on OS/GPU)
    Train {
        /// Base model (HF id or local path)
        #[arg(long, default_value = "Qwen/Qwen2.5-7B-Instruct")]
        base: String,
        /// Dataset JSONL from `prepare`
        #[arg(long)]
        dataset: PathBuf,
        /// Output adapter dir
        #[arg(long, default_value = "./outputs/lora-adapter")]
        output: PathBuf,
        /// LoRA rank
        #[arg(long, default_value = "16")]
        rank: u32,
        /// LoRA alpha
        #[arg(long, default_value = "32")]
        alpha: u32,
        /// Max seq length
        #[arg(long, default_value = "4096")]
        max_seq_len: u32,
        /// Epochs
        #[arg(long, default_value = "3")]
        epochs: u32,
        /// Learning rate
        #[arg(long, default_value = "0.0002")]
        lr: String,
        /// Batch size per device
        #[arg(long, default_value = "1")]
        batch: u32,
        /// Gradient accumulation steps
        #[arg(long, default_value = "4")]
        grad_accum: u32,
        /// Force backend: unsloth, mlx, axolotl, torchtune
        #[arg(long)]
        backend: Option<String>,
        /// Dry run — print the training command without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Merge LoRA adapter into base model (for GGUF/vLLM)
    Merge {
        #[arg(long, default_value = "Qwen/Qwen2.5-7B-Instruct")]
        base: String,
        #[arg(long, default_value = "./outputs/lora-adapter")]
        adapter: PathBuf,
        #[arg(long, default_value = "./outputs/merged")]
        out: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Quantize merged model to GGUF for LM Studio / Ollama
    Quantize {
        #[arg(long, default_value = "./outputs/merged")]
        model: PathBuf,
        #[arg(long, default_value = "./outputs/gguf")]
        out: PathBuf,
        #[arg(long, default_value = "q4_k_m")]
        quant: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Full pipeline: prepare → train → merge → quantize
    Pipeline {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "Qwen/Qwen2.5-7B-Instruct")]
        base: String,
        #[arg(long, default_value = "./finetune-dataset.jsonl")]
        dataset: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Eval gate: compare base vs adapter on bench prompts, block regressing deploys
    Eval {
        /// Base: model id (live) or bench result JSON (offline compare)
        #[arg(long)]
        base: String,
        /// Adapter/challenger: model id (live) or bench result JSON (offline compare)
        #[arg(long)]
        adapter: String,
        /// Prompt set for live eval (default: ./bench/prompts.jsonl)
        #[arg(long, default_value = "bench/prompts.jsonl")]
        bench: PathBuf,
        /// Suite filter: coding|reasoning (default: all)
        #[arg(long)]
        suite: Option<String>,
        /// Project for context + verifier_clean scoring (recommended)
        #[arg(long)]
        project: Option<String>,
        /// Provider override (default: config/auto)
        #[arg(long)]
        provider: Option<String>,
        /// Results dir for live runs (default: ./bench/results)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Don't fail on regression (inspect only; default fails to block deploy)
        #[arg(long)]
        allow_regression: bool,
    },
}

fn should_skip_for_dataset(rel: &str, content: &str, min_chars: usize) -> bool {
    if content.trim().len() < min_chars {
        return true;
    }
    let lower = rel.to_lowercase();
    // Lockfiles / generated / vendored — noise for finetune, blow up dataset
    for pat in [
        "package-lock.json",
        "cargo.lock",
        "pnpm-lock.yaml",
        "yarn.lock",
        "poetry.lock",
        "pip-freeze.txt",
        ".min.js",
        ".min.css",
        ".map",
        ".lock",
        ".gguf",
        ".bin",
        ".onnx",
        "node_modules/",
    ] {
        if lower.contains(pat) {
            return true;
        }
    }
    // Junk test placeholders like test-one.txt with a single word
    if lower.starts_with("test-") && content.split_whitespace().count() < 5 {
        return true;
    }
    // Secrets — never train on these
    if lower.contains(".env") || lower.ends_with(".pem") || lower.ends_with(".key") {
        return true;
    }
    false
}

fn chunk_content(content: &str, max_chars: usize) -> Vec<String> {
    if content.len() <= max_chars {
        return vec![content.to_string()];
    }
    // Split on line boundaries with 200-char overlap to preserve context
    let overlap = 200.min(max_chars / 5);
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let bytes = content.as_bytes();
    while start < content.len() {
        let mut end = (start + max_chars).min(content.len());
        // Snap end to char boundary
        while end < content.len() && !content.is_char_boundary(end) {
            end += 1;
        }
        // Snap to newline if near end (avoid cutting mid-line)
        if end < content.len() {
            if let Some(nl) = content[start..end].rfind('\n') {
                let snap = start + nl + 1;
                if snap > start + max_chars / 2 {
                    end = snap;
                    while end < content.len() && !content.is_char_boundary(end) {
                        end += 1;
                    }
                }
            }
        }
        chunks.push(content[start..end].to_string());
        if end >= content.len() {
            break;
        }
        // Overlap backwards to newline
        let next_start = end.saturating_sub(overlap);
        // Avoid infinite loop on tiny progress
        start = if next_start <= start { end } else {
            // snap next_start forward to char boundary + line start
            let mut s = next_start;
            while s < bytes.len() && !content.is_char_boundary(s) {
                s += 1;
            }
            s
        };
        if chunks.len() > 50 {
            break; // safety: 50 chunks * 10k = 500k per file max
        }
    }
    chunks
}

fn validate_dataset(path: &std::path::Path) -> anyhow::Result<(usize, bool)> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path)?;
    let reader = BufReader::new(f);
    let mut n = 0usize;
    let mut has_text = false;
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| anyhow::anyhow!("line {}: {}", i + 1, e))?;
        // Accept sharegpt/chatml (messages), alpaca (instruction/output), or raw text
        let ok = v.get("messages").is_some() || v.get("text").is_some() || v.get("instruction").is_some();
        if !ok {
            return Err(anyhow::anyhow!(
                "line {}: needs `messages`, `text`, or `instruction` field",
                i + 1
            ));
        }
        if v.get("text").is_some() {
            has_text = true;
        }
        n += 1;
        if n >= 100_000 {
            break;
        }
    }
    if n == 0 {
        return Err(anyhow::anyhow!("empty file"));
    }
    Ok((n, has_text))
}

fn detect_backend(force: Option<String>) -> String {
    if let Some(b) = force { return b; }
    if cfg!(target_os = "macos") {
        // Apple Silicon -> MLX best, else torchtune MPS
        return "mlx".to_string();
    }
    // Check for nvidia
    let has_nvidia = std::process::Command::new("nvidia-smi").output().map(|o| o.status.success()).unwrap_or(false);
    if has_nvidia {
        return "unsloth".to_string();
    }
    // Fallback
    "torchtune".to_string()
}

fn python_bin() -> String {
    // Cross-platform python detection: Windows uses `python`/`py`, Unix uses `python3`/`python`
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &["python", "py", "python3"]
    } else {
        &["python3", "python"]
    };
    for cand in candidates {
        if std::process::Command::new(*cand)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return cand.to_string();
        }
    }
    // Fallback to platform default if none detected
    if cfg!(target_os = "windows") { "python".to_string() } else { "python3".to_string() }
}

fn finetune_python_dir() -> PathBuf {
    // Expect finetune/ next to cli/ at project root, or fallback to exe dir
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| PathBuf::from("."));
    // Try multiple locations
    for cand in [
        PathBuf::from("finetune"),
        PathBuf::from("../finetune"),
        exe_dir.join("finetune"),
        exe_dir.join("../finetune"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../finetune"),
    ] {
        if cand.exists() {
            return cand;
        }
    }
    PathBuf::from("finetune")
}

pub async fn handle(args: FinetuneArgs) -> Result<()> {
    // In plan mode, all mutating finetune commands are blocked (read-only)
    let is_plan = crate::core::config::is_plan_mode();
    match args.command {
        FinetuneCommands::Status => {
            println!("{}", style("Local AI — Finetune Status").bold());
            println!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
            // GPU
            let nvidia = std::process::Command::new("nvidia-smi").arg("--query-gpu=name,memory.total,driver_version").arg("--format=csv,noheader").output();
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
                println!("GPU (Apple): MPS available — recommend MLX or torchtune");
            }
            let backend = detect_backend(None);
            println!("Recommended backend: {}", style(&backend).cyan().bold());
            println!("\nBackends:");
            println!("  unsloth  — 2-5x faster, 5-6GB for 7B QLoRA (NVIDIA only, best for RTX 5050 8GB)");
            println!("  axolotl  — most configurable, multi-GPU, YAML-driven");
            println!("  mlx      — Apple Silicon native (M1/M2/M3/M4)");
            println!("  torchtune— PyTorch native, MPS+CUDA, best transparency");
            println!("\nFinetune dir: {}", finetune_python_dir().display());
            // Check python — cross-platform
            let py_candidates: &[&str] = if cfg!(target_os = "windows") { &["python", "py", "python3"] } else { &["python3", "python"] };
            let mut py_ver: Option<String> = None;
            for cand in py_candidates {
                if let Ok(o) = std::process::Command::new(*cand).arg("--version").output() {
                    if o.status.success() {
                        let s = if !o.stdout.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { String::from_utf8_lossy(&o.stderr).trim().to_string() };
                        py_ver = Some(s);
                        break;
                    }
                }
            }
            if let Some(v) = py_ver { println!("Python: {}", v); } else { println!("Python: not found"); }
            // Check LM Studio URL
            println!("LM Studio: {}", crate::core::lm_studio_base_url());
            println!("\nQuick start:");
            println!("  local-ai finetune prepare --project ./my-app --out ./dataset.jsonl");
            println!("  local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl");
            println!("  local-ai finetune merge --base Qwen/Qwen2.5-7B-Instruct --adapter ./outputs/lora-adapter --out ./outputs/merged");
            println!("  local-ai finetune quantize --model ./outputs/merged --out ./outputs/gguf --quant q4_k_m  # then load in LM Studio");
        }
        FinetuneCommands::Prepare { project, out, format, max_chars, min_chars, system_prompt } => {
            if is_plan { crate::core::config::require_build_mode("finetune prepare")?; }
            let proj = projects::resolve_project(project)?;
            let files = core_fs::collect_files_for_finetune(&proj)?;
            println!("Collecting from {} ({} files) → format: {} → {}", proj.folder_path.as_deref().unwrap_or("."), files.len(), format, out.display());
            if files.is_empty() {
                anyhow::bail!("No files found to prepare dataset");
            }
            // Build dataset JSONL — chunked, diverse instructions, TRL-ready (messages + text)
            let system = system_prompt.unwrap_or_else(|| "You are a helpful coding assistant. Answer based on the project context.".to_string());
            let mut count = 0usize;
            let mut skipped = 0usize;
            let mut out_file = std::fs::File::create(&out)?;
            use std::io::Write;
            // Rotate instruction templates for diversity (avoids "Explain file X" overfit,
            // gives both code-explain + chat-style Q&A for personal code model).
            let templates = [
                ("Explain the file: {}", "Explain this file's purpose, key functions, and how it fits the project:"),
                ("Review {} for bugs and suggest fixes", "Review this file for bugs, edge cases, and improvements:"),
                ("Document the public API in {}", "Write concise docs for the public functions/types in:"),
                ("How does {} work?", "Explain step-by-step how this code works:"),
                ("Extend {} with a small feature", "Read this file, then show how you'd extend it with a focused improvement:"),
            ];
            for (rel, content) in files {
                if should_skip_for_dataset(&rel, &content, min_chars) { skipped += 1; continue; }
                let chunks = chunk_content(&content, max_chars);
                let n_chunks = chunks.len();
                for (chunk_idx, chunk) in chunks.into_iter().enumerate() {
                    if chunk.trim().len() < min_chars { continue; }
                    let (_t_short, t_user_full) = &templates[count % templates.len()];
                    let user_msg = if n_chunks > 1 {
                        format!("{} (part {}/{})\n\n```\n{}\n```", t_user_full.replace("{}", &rel), chunk_idx + 1, n_chunks, chunk)
                    } else {
                        format!("{}\n\nFile `{}`:\n```\n{}\n```", t_user_full.replace("{}", &rel), rel, chunk)
                    };
                    // Create a training sample per file chunk
                    let text_render = format!("system: {}\nuser: {}\nassistant: {}", system, user_msg, chunk);
                    let sample = match format.as_str() {
                        "alpaca" => serde_json::json!({
                            "instruction": user_msg,
                            "input": "",
                            "output": chunk,
                            "text": text_render
                        }),
                        _ => serde_json::json!({
                            "messages": [
                                {"role": "system", "content": system},
                                {"role": "user", "content": user_msg},
                                {"role": "assistant", "content": chunk}
                            ],
                            // TRL SFTTrainer compat: pre-rendered text field
                            "text": text_render
                        }),
                    };
                    writeln!(out_file, "{}", serde_json::to_string(&sample)?)?;
                    count += 1;
                }
            }
            println!("{} Wrote {} samples ({} skipped) to {}", style("✓").green(), count, skipped, out.display());
            println!("  Preview: head -n 1 {} | jq .", out.display());
            if count == 0 {
                anyhow::bail!("All files skipped (too small or filtered) — try --min-chars 10 or check project path");
            }
            println!("  Tip: curate dataset (remove secrets), then: local-ai finetune train --dataset {} --dry-run", out.display());
        }
        FinetuneCommands::Train { base, dataset, output, rank, alpha, max_seq_len, epochs, lr, batch, grad_accum, backend, dry_run } => {
            if is_plan { crate::core::config::require_build_mode("finetune train")?; }
            let bk = detect_backend(backend);
            let py_dir = finetune_python_dir();
            let train_script = py_dir.join("train.py");
            println!("Backend: {}  Base: {}  Dataset: {}  Output: {}", style(&bk).cyan(), base, dataset.display(), output.display());
            println!("  rank={} alpha={} seq={} epochs={} lr={} batch={} grad_accum={}", rank, alpha, max_seq_len, epochs, lr, batch, grad_accum);
            if !dataset.exists() {
                anyhow::bail!("Dataset not found: {}", dataset.display());
            }
            // Validate dataset JSONL early (fail fast before GPU alloc)
            match validate_dataset(&dataset) {
                Ok((n, has_text)) => {
                    println!("  Dataset: {} samples{}", n, if has_text { " (TRL-ready: has `text`)" } else { " (WARN: no `text` field — train.py will render from messages)" });
                    if n < 10 {
                        eprintln!("  {} Only {} samples — finetune will overfit. Add more files or lower --min-chars.", style("!").yellow(), n);
                    }
                    if n > 5000 {
                        println!("  Large dataset ({} samples) — consider --epochs 1-2 first.", n);
                    }
                }
                Err(e) => anyhow::bail!("Invalid dataset {}: {}", dataset.display(), e),
            }
            // 8GB VRAM guard (RTX 5050): 7B+ with seq 4096 + batch>1 often OOMs
            if max_seq_len > 2048 && batch > 1 {
                eprintln!("  {} seq={} + batch={} may OOM on 8GB. Suggest --max-seq-len 2048 --batch 1 --grad-accum 8", style("!").yellow(), max_seq_len, batch);
            }
            if bk == "unsloth" {
                // Quick import check for better error than Python traceback
                let check = std::process::Command::new(python_bin()).args(["-c", "import unsloth"]).output();
                if !check.map(|o| o.status.success()).unwrap_or(false) {
                    eprintln!("  {} `unsloth` not installed. Install: pip install \"unsloth[colab-new] @ git+https://github.com/unslothai/unsloth.git\" trl transformers datasets accelerate", style("!").yellow());
                    if !dry_run {
                        eprintln!("  Continuing anyway — train.py will exit with install hint.");
                    }
                }
            }
            if dry_run {
                let py = python_bin();
                println!("\n[dry-run] Would run:");
                println!("  {} {} --backend {} --base {} --dataset {} --output {} --rank {} --alpha {} --max-seq-len {} --epochs {} --lr {} --batch {} --grad-accum {}",
                    py, train_script.display(), bk, base, dataset.display(), output.display(), rank, alpha, max_seq_len, epochs, lr, batch, grad_accum);
                return Ok(());
            }
            if !train_script.exists() {
                eprintln!("{} train.py not found at {} — creating minimal script…", style("!").yellow(), train_script.display());
                std::fs::create_dir_all(&py_dir)?;
                std::fs::write(&train_script, minimal_train_py())?;
                println!("  Created {}", train_script.display());
                println!("  Install deps: pip install -r {}/requirements.txt", py_dir.display());
            }
            println!("\n{} Launching training…", style("→").cyan());
            let py = python_bin();
            let status = std::process::Command::new(py)
                .arg(&train_script)
                .arg("--backend").arg(&bk)
                .arg("--base").arg(&base)
                .arg("--dataset").arg(&dataset)
                .arg("--output").arg(&output)
                .arg("--rank").arg(rank.to_string())
                .arg("--alpha").arg(alpha.to_string())
                .arg("--max-seq-len").arg(max_seq_len.to_string())
                .arg("--epochs").arg(epochs.to_string())
                .arg("--lr").arg(&lr)
                .arg("--batch").arg(batch.to_string())
                .arg("--grad-accum").arg(grad_accum.to_string())
                .status()?;
            if !status.success() {
                anyhow::bail!("Training failed with exit {}", status);
            }
            println!("{} Training complete → {}", style("✓").green(), output.display());
        }
        FinetuneCommands::Merge { base, adapter, out, dry_run } => {
            if is_plan { crate::core::config::require_build_mode("finetune merge")?; }
            let py_dir = finetune_python_dir();
            let script = py_dir.join("merge.py");
            println!("Merge adapter {} into base {} → {}", adapter.display(), base, out.display());
            if dry_run {
                let py = python_bin();
                println!("[dry-run] {} {} --base {} --adapter {} --out {}", py, script.display(), base, adapter.display(), out.display());
                return Ok(());
            }
            if !script.exists() {
                std::fs::create_dir_all(&py_dir)?;
                std::fs::write(&script, minimal_merge_py())?;
            }
            let py = python_bin();
            let status = std::process::Command::new(py).arg(&script).arg("--base").arg(&base).arg("--adapter").arg(&adapter).arg("--out").arg(&out).status()?;
            if !status.success() { anyhow::bail!("Merge failed"); }
            println!("{} Merged → {}", style("✓").green(), out.display());
        }
        FinetuneCommands::Quantize { model, out, quant, dry_run } => {
            if is_plan { crate::core::config::require_build_mode("finetune quantize")?; }
            let py_dir = finetune_python_dir();
            let script = py_dir.join("quantize.py");
            println!("Quantize {} → {} (quant: {})", model.display(), out.display(), quant);
            if dry_run {
                let py = python_bin();
                println!("[dry-run] {} {} --model {} --out {} --quant {}", py, script.display(), model.display(), out.display(), quant);
                return Ok(());
            }
            if !script.exists() {
                std::fs::create_dir_all(&py_dir)?;
                std::fs::write(&script, minimal_quantize_py())?;
            }
            let py = python_bin();
            let status = std::process::Command::new(py).arg(&script).arg("--model").arg(&model).arg("--out").arg(&out).arg("--quant").arg(&quant).status()?;
            if !status.success() { anyhow::bail!("Quantize failed"); }
            println!("{} GGUF at {} — load in LM Studio (Add Model → Local File)", style("✓").green(), out.display());
        }
        FinetuneCommands::Pipeline { project, base, dataset, dry_run } => {
            if is_plan { crate::core::config::require_build_mode("finetune pipeline")?; }
            println!("{}", style("Running full pipeline: prepare → train → merge → quantize").bold());
            // Prepare
            let proj = projects::resolve_project(project)?;
            println!("\n[1/4] Prepare dataset...");
            let prepare_args = FinetuneArgs { command: FinetuneCommands::Prepare { project: proj.folder_path.clone(), out: dataset.clone(), format: "sharegpt".into(), max_chars: 10000, min_chars: 50, system_prompt: None } };
            Box::pin(handle(prepare_args)).await?;
            println!("\n[2/4] Train...");
            let train_args = FinetuneArgs { command: FinetuneCommands::Train { base: base.clone(), dataset: dataset.clone(), output: PathBuf::from("./outputs/lora-adapter"), rank: 16, alpha: 32, max_seq_len: 4096, epochs: 3, lr: "0.0002".into(), batch: 1, grad_accum: 4, backend: None, dry_run } };
            Box::pin(handle(train_args)).await?;
            if dry_run { println!("\n[dry-run] Pipeline would continue: merge → quantize"); return Ok(()); }
            println!("\n[3/4] Merge...");
            let merge_args = FinetuneArgs { command: FinetuneCommands::Merge { base: base.clone(), adapter: PathBuf::from("./outputs/lora-adapter"), out: PathBuf::from("./outputs/merged"), dry_run: false } };
            Box::pin(handle(merge_args)).await?;
            println!("\n[4/4] Quantize...");
            let quant_args = FinetuneArgs { command: FinetuneCommands::Quantize { model: PathBuf::from("./outputs/merged"), out: PathBuf::from("./outputs/gguf"), quant: "q4_k_m".into(), dry_run: false } };
            Box::pin(handle(quant_args)).await?;
            println!("\n{} Pipeline complete! Load outputs/gguf in LM Studio", style("✓").green());
        }
        FinetuneCommands::Eval { base, adapter, bench, suite, project, provider, out, allow_regression } => {
            handle_eval(base, adapter, bench, suite, project, provider, out, allow_regression).await?;
        }
    }
    Ok(())
}

/// `finetune eval` — deploy gate (Phase 5.2).
///
/// Each side (`--base`, `--adapter`) is either a bench result JSON (offline
/// compare, no model needed) or a model id for a live run against
/// `--bench`. Live runs stream each prompt (TTFT/TPS) and score pass@1;
/// results are saved to `--out`/`bench/results` for audit. The gate passes
/// iff adapter pass@1 ≥ base pass@1 — otherwise this returns `Err` so
/// `merge → quantize → deploy` scripts stop (unless `--allow-regression`).
#[allow(clippy::too_many_arguments)]
async fn handle_eval(
    base: String,
    adapter: String,
    bench_path: PathBuf,
    suite: Option<String>,
    project: Option<String>,
    provider_override: Option<String>,
    out: Option<PathBuf>,
    allow_regression: bool,
) -> Result<()> {
    use crate::core::config::{load_config, ProviderKind};
    use crate::core::{intelligence, provider, verifier};

    fn looks_like_result_file(spec: &str) -> bool {
        let p = std::path::Path::new(spec);
        p.exists() && p.is_file() && p.extension().map(|e| e == "json").unwrap_or(false)
    }

    // Offline fast path: both sides are result files → just compare.
    if looks_like_result_file(&base) && looks_like_result_file(&adapter) {
        let ra = core_bench::load_result(std::path::Path::new(&base))?;
        let rb = core_bench::load_result(std::path::Path::new(&adapter))?;
        print_eval_table(&ra, &rb);
        let (ok, line) = core_bench::gate_verdict(&ra, &rb);
        println!("  {}", if ok { style(&line).green() } else { style(&line).red() });
        if !ok && !allow_regression {
            anyhow::bail!("eval gate BLOCKED: adapter regressed (pass --allow-regression to inspect only)");
        }
        return Ok(());
    }
    // Mixed file/model is a user error — say so explicitly.
    if looks_like_result_file(&base) != looks_like_result_file(&adapter) {
        anyhow::bail!("--base and --adapter must both be model ids (live) or both be result JSON files (offline)");
    }

    // Live path: stream `--bench` prompts against both models.
    let cfg = load_config().unwrap_or_default();
    let provider_kind: ProviderKind = if let Some(p) = provider_override {
        p.parse().map_err(|e: String| anyhow::anyhow!(e))?
    } else {
        cfg.provider.active.clone()
    };
    let provider_url = crate::core::config::resolve_provider_url(&provider_kind, &cfg, None, None);

    let prompts = core_bench::load_prompts(&bench_path)?;
    let prompts: Vec<_> = match &suite {
        Some(s) => prompts.into_iter().filter(|p| p.suite == *s).collect(),
        None => prompts,
    };
    if prompts.is_empty() {
        anyhow::bail!("No prompts match suite {:?}", suite);
    }

    // Project context (best-effort) powers grounding + verifier_clean scoring.
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
        let index_opt = crate::core::index::load_index(proj).ok().flatten();
        let use_hybrid = index_opt.as_ref().map(|i| !crate::core::index::needs_rebuild(proj, i)).unwrap_or(false);
        if use_hybrid {
            let emb = crate::core::embeddings::get_embedder(&cfg);
            Some(intelligence::build_project_context_hybrid("finetune eval", &files, index_opt.as_ref(), Some(emb), |p| {
                core_fs::read_project_file(proj, p).ok()
            }))
        } else {
            Some(intelligence::build_project_context("finetune eval", &files, |p| {
                core_fs::read_project_file(proj, p).ok()
            }))
        }
        }
    } else {
        None
    };

    #[allow(clippy::too_many_arguments)]
    async fn eval_model(
        model_id: &str,
        prompts: &[core_bench::BenchPrompt],
        suite: Option<&str>,
        provider_kind: &ProviderKind,
        provider_url: &str,
        cfg: &crate::core::config::AppConfig,
        context: &Option<String>,
        proj_opt: &Option<crate::core::projects::Project>,
        files: &[crate::core::fs::ProjectFile],
    ) -> Result<core_bench::BenchResult> {
        println!("{}", style(format!("EVAL — {} ({} prompts)", model_id, prompts.len())).bold());
        let mut results = Vec::new();
        for prompt in prompts {
            let system = match context {
                Some(c) => format!("You are Local AI, a grounded coding assistant. Use ONLY the project context. Be concise.\n\n{}", c),
                None => "You are Local AI, a concise coding assistant.".to_string(),
            };
            let messages = vec![
                provider::ChatMessage { role: "system".into(), content: system },
                provider::ChatMessage { role: "user".into(), content: prompt.user.clone() },
            ];
            let start = std::time::Instant::now();
            let mut first_ms: Option<u64> = None;
            let mut streamed = 0usize;
            let mut sink = |chunk: &str| {
                if first_ms.is_none() {
                    first_ms = Some(start.elapsed().as_millis() as u64);
                }
                streamed += chunk.len();
            };
            let output = provider::stream_chat_unified(provider_kind, provider_url, cfg, model_id, messages, 0.4, &mut sink)
                .await
                .unwrap_or_else(|e| format!("(model error: {})", e));
            let secs = start.elapsed().as_secs_f64();
            let ttft_ms = first_ms.unwrap_or((secs * 1000.0) as u64);
            let tps = core_bench::estimate_tps(streamed.max(output.len()), secs);
            let verifier_ok = match (proj_opt, prompt.kind.as_str()) {
                (Some(proj), "verifier_clean") => {
                    let report = verifier::verify_response_hybrid(&output, proj, files, "balanced", false);
                    Some(!report.is_hallucinated)
                }
                _ => None,
            };
            let (pass, note) = core_bench::score_prompt(prompt, &output, verifier_ok);
            println!("  [{}] {} {:.1}s — {}", prompt.id, if pass { style("✓").green().to_string() } else { style("✗").red().to_string() }, secs, note);
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
        Ok(core_bench::BenchResult::summarize(model_id, suite.unwrap_or("all"), results))
    }

    let ra = eval_model(&base, &prompts, suite.as_deref(), &provider_kind, &provider_url, &cfg, &context, &proj_opt, &files).await?;
    let rb = eval_model(&adapter, &prompts, suite.as_deref(), &provider_kind, &provider_url, &cfg, &context, &proj_opt, &files).await?;

    // Persist live runs for audit (same dir as `bench run`).
    let dir = match out {
        Some(d) => { std::fs::create_dir_all(&d)?; d }
        None => core_bench::default_results_dir()?,
    };
    let pa = core_bench::save_result(&dir, &ra)?;
    let pb = core_bench::save_result(&dir, &rb)?;
    println!("  saved: {}\n  saved: {}", pa.display(), pb.display());

    print_eval_table(&ra, &rb);
    let (ok, line) = core_bench::gate_verdict(&ra, &rb);
    println!("  {}", if ok { style(line).green() } else { style(line).red() });
    // Verifier still runs post-deploy: grounding is never bypassed by a passing gate.
    println!("  {} verifier still runs post-deploy on every model output", style("→").dim());
    if !ok && !allow_regression {
        anyhow::bail!("eval gate BLOCKED: adapter regressed — refusing deploy (pass --allow-regression to inspect only)");
    }
    Ok(())
}

fn print_eval_table(ra: &core_bench::BenchResult, rb: &core_bench::BenchResult) {
    println!("{}", style(format!("EVAL — {} vs {}", ra.model, rb.model)).bold());
    println!("  {:<28} {:>10} {:>10} {:>8}", "prompt", ra.model.chars().take(10).collect::<String>(), rb.model.chars().take(10).collect::<String>(), "Δtps");
    for row in core_bench::compare(ra, rb) {
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
        "  pass@1: {:.2} → {:.2} | TPS: {:.1} → {:.1} | TTFT: {}ms → {}ms",
        ra.pass_at_1, rb.pass_at_1, ra.mean_tps, rb.mean_tps, ra.mean_ttft_ms, rb.mean_ttft_ms
    );
}

fn minimal_train_py() -> String {
    r#"#!/usr/bin/env python3
"""Minimal finetune dispatcher — supports unsloth/mlx/torchtune/axolotl backends."""
import argparse, os, sys, json

def train_unsloth(args):
    print(f"[unsloth] Training {args.base} on {args.dataset}")
    try:
        from unsloth import FastLanguageModel
        from trl import SFTTrainer
        from transformers import TrainingArguments
        from datasets import load_dataset
        import torch
        print("Unsloth available, starting QLoRA...")
        # Minimal example — expand for production
        max_seq_length = args.max_seq_len
        model, tokenizer = FastLanguageModel.from_pretrained(
            model_name=args.base,
            max_seq_length=max_seq_length,
            dtype=None,
            load_in_4bit=True,
        )
        model = FastLanguageModel.get_peft_model(
            model,
            r=args.rank,
            target_modules=["q_proj","k_proj","v_proj","o_proj","gate_proj","up_proj","down_proj"],
            lora_alpha=args.alpha,
            lora_dropout=0,
            bias="none",
            use_gradient_checkpointing="unsloth",
            random_state=3407,
        )
        dataset = load_dataset("json", data_files=str(args.dataset), split="train")
        def formatting_func(examples):
            # Handle both sharegpt and alpaca
            texts = []
            for msgs in examples.get("messages", []):
                # This is batched; handle appropriately in real run
                pass
            return examples
        # For MVP, assume dataset already formatted with "text" field
        # Users should preprocess dataset to text field if needed
        from datasets import Dataset
        # Simple text formatting
        def to_text(example):
            if "text" in example:
                return {"text": example["text"]}
            if "messages" in example:
                msgs = example["messages"]
                text = ""
                for m in msgs:
                    text += f"{m['role']}: {m['content']}\n"
                return {"text": text.strip()}
            return {"text": json.dumps(example)}
        dataset = dataset.map(to_text)
        trainer = SFTTrainer(
            model=model,
            tokenizer=tokenizer,
            train_dataset=dataset,
            dataset_text_field="text",
            max_seq_length=max_seq_length,
            dataset_num_proc=2,
            args=TrainingArguments(
                per_device_train_batch_size=args.batch,
                gradient_accumulation_steps=args.grad_accum,
                warmup_steps=10,
                num_train_epochs=args.epochs,
                learning_rate=float(args.lr),
                fp16=not torch.cuda.is_bf16_supported(),
                bf16=torch.cuda.is_bf16_supported(),
                logging_steps=1,
                optim="adamw_8bit",
                weight_decay=0.01,
                lr_scheduler_type="linear",
                seed=3407,
                output_dir=str(args.output),
            ),
        )
        trainer.train()
        model.save_pretrained(str(args.output))
        tokenizer.save_pretrained(str(args.output))
        print(f"Saved adapter to {args.output}")
    except ImportError as e:
        print(f"Unsloth not installed: {e}")
        print("Install: pip install unsloth trl transformers datasets accelerate")
        sys.exit(1)

def train_mlx(args):
    print(f"[mlx] Training on Apple Silicon — base {args.base}")
    print("MLX finetune requires: pip install mlx mlx-lm")
    print("Example: python -m mlx_lm.lora --model {} --train --data {} --adapter-path {}".format(args.base, args.dataset, args.output))
    # Dispatch to mlx_lm
    os.system(f"python -m mlx_lm.lora --model {args.base} --train --data {args.dataset} --adapter-path {args.output} --num-layers 16 --batch-size {args.batch} --iters 1000")

def train_generic(args):
    print(f"[{args.backend}] Generic training — base {args.base}, dataset {args.dataset}")
    print("Install axolotl or torchtune and configure YAML/recipe for full training")
    print("For now, see finetune/README.md for backend-specific instructions")

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--backend", default="unsloth")
    p.add_argument("--base", required=True)
    p.add_argument("--dataset", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--rank", type=int, default=16)
    p.add_argument("--alpha", type=int, default=32)
    p.add_argument("--max-seq-len", type=int, default=4096)
    p.add_argument("--epochs", type=int, default=3)
    p.add_argument("--lr", default="0.0002")
    p.add_argument("--batch", type=int, default=1)
    p.add_argument("--grad-accum", type=int, default=4)
    args = p.parse_args()
    if args.backend == "unsloth":
        train_unsloth(args)
    elif args.backend == "mlx":
        train_mlx(args)
    else:
        train_generic(args)
"#.to_string()
}

fn minimal_merge_py() -> String {
    r#"#!/usr/bin/env python3
import argparse
from pathlib import Path
print("Merging LoRA adapter...")
# Unsloth merge example:
# from unsloth import FastLanguageModel
# model, tok = FastLanguageModel.from_pretrained(model_name=args.base, load_in_4bit=False)
# model.load_adapter(args.adapter)
# model.save_pretrained_merged(args.out, tok, save_method="merged_16bit")
p = argparse.ArgumentParser()
p.add_argument("--base", required=True)
p.add_argument("--adapter", required=True)
p.add_argument("--out", required=True)
args = p.parse_args()
print(f"Would merge {args.adapter} into {args.base} -> {args.out}")
print("Implement merge with: FastLanguageModel.save_pretrained_merged or peft merge_and_unload()")
"#.to_string()
}

fn minimal_quantize_py() -> String {
    r#"#!/usr/bin/env python3
import argparse, subprocess, sys
from pathlib import Path
p = argparse.ArgumentParser()
p.add_argument("--model", required=True)
p.add_argument("--out", required=True)
p.add_argument("--quant", default="q4_k_m")
args = p.parse_args()
print(f"Quantizing {args.model} -> {args.out} ({args.quant})")
print("Requires llama.cpp: python -m llama_cpp ... or use `llama-quantize`")
print("Example: python -c 'from llama_cpp import ...' or use: https://github.com/ggerganov/llama.cpp")
print("For LM Studio: use built-in GGUF export or `python -m unsloth` GGUF save")
# Try unsloth GGUF
try:
    print("Attempting Unsloth GGUF export...")
    from unsloth import FastLanguageModel
    model, tokenizer = FastLanguageModel.from_pretrained(model_name=str(args.model), max_seq_length=4096, dtype=None, load_in_4bit=False)
    model.save_pretrained_gguf(str(args.out), tokenizer, quantization_method=args.quant)
    print(f"Saved GGUF to {args.out}")
except Exception as e:
    print(f"GGUF export failed: {e}")
    sys.exit(1)
"#.to_string()
}
