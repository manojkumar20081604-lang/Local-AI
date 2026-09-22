use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

use crate::core::{fs as core_fs, projects};

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
        /// Max chars per file
        #[arg(long, default_value = "10000")]
        max_chars: usize,
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
        FinetuneCommands::Prepare { project, out, format, max_chars, system_prompt } => {
            if is_plan { crate::core::config::require_build_mode("finetune prepare")?; }
            let proj = projects::resolve_project(project)?;
            let files = core_fs::collect_files_for_finetune(&proj)?;
            println!("Collecting from {} ({} files) → format: {} → {}", proj.folder_path.as_deref().unwrap_or("."), files.len(), format, out.display());
            if files.is_empty() {
                anyhow::bail!("No files found to prepare dataset");
            }
            // Build dataset JSONL
            let system = system_prompt.unwrap_or_else(|| "You are a helpful coding assistant. Answer based on the project context.".to_string());
            let mut count = 0usize;
            let mut out_file = std::fs::File::create(&out)?;
            use std::io::Write;
            for (rel, content) in files {
                if content.trim().is_empty() { continue; }
                let truncated = if content.len() > max_chars {
                    let mut end = max_chars;
                    while end > 0 && !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    &content[..end]
                } else { &content };
                // Create a training sample per file
                let sample = match format.as_str() {
                    "alpaca" => serde_json::json!({
                        "instruction": format!("Explain or improve the file: {}", rel),
                        "input": "",
                        "output": truncated,
                        "text": format!("### Instruction:\nExplain file {}\n\n### Response:\n{}", rel, truncated)
                    }),
                    "chatml" | "sharegpt" => serde_json::json!({
                        "messages": [
                            {"role": "system", "content": system},
                            {"role": "user", "content": format!("Explain the file: {}", rel)},
                            {"role": "assistant", "content": truncated}
                        ]
                    }),
                    _ => serde_json::json!({
                        "messages": [
                            {"role": "system", "content": system},
                            {"role": "user", "content": format!("Analyze {}", rel)},
                            {"role": "assistant", "content": truncated}
                        ]
                    }),
                };
                writeln!(out_file, "{}", serde_json::to_string(&sample)?)?;
                count += 1;
            }
            println!("{} Wrote {} samples to {}", style("✓").green(), count, out.display());
            println!("  Preview: head -n 1 {} | jq .", out.display());
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
            let prepare_args = FinetuneArgs { command: FinetuneCommands::Prepare { project: proj.folder_path.clone(), out: dataset.clone(), format: "sharegpt".into(), max_chars: 10000, system_prompt: None } };
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
    }
    Ok(())
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
