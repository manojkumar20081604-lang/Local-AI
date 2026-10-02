#!/usr/bin/env python3
"""
Local-AI finetune dispatcher — Unsloth (NVIDIA) / MLX (Apple) / torchtune / Axolotl.

NOTE: Finetuned model still goes through same verifier (cli/src/core/verifier.rs +
finetune/verify.py) — finetuning does NOT bypass grounding. All responses are
post-verified for hallucinations. Finetuning improves style/domain, not factual grounding.

Dataset: JSONL from `local-ai finetune prepare` — each line has `messages` + `text`.
Supports sharegpt/chatml (messages), alpaca (instruction/input/output), raw text.

8GB-safe defaults (RTX 5050): rank 16, seq 2048-4096, batch 1, grad-accum 4-8,
4-bit QLoRA, gradient checkpointing (unsloth), adamw_8bit.
"""
from __future__ import annotations
import argparse
import json
import os
import sys
from pathlib import Path


def load_jsonl(path: Path):
    rows = []
    with open(path, encoding="utf-8") as f:
        for i, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as e:
                print(f"[warn] line {i}: skip invalid JSON ({e})", file=sys.stderr)
    return rows


def to_text_example(ex: dict) -> str:
    """Render any dataset row to plain text for SFTTrainer."""
    if isinstance(ex.get("text"), str) and ex["text"].strip():
        return ex["text"].strip()
    if isinstance(ex.get("messages"), list):
        parts = []
        for m in ex["messages"]:
            role = m.get("role", "user")
            content = m.get("content", "")
            parts.append(f"{role}: {content}")
        return "\n".join(parts).strip()
    if "instruction" in ex:
        instr = ex.get("instruction", "")
        inp = ex.get("input", "")
        out = ex.get("output", "")
        if inp:
            return f"### Instruction:\n{instr}\n\n### Input:\n{inp}\n\n### Response:\n{out}"
        return f"### Instruction:\n{instr}\n\n### Response:\n{out}"
    return json.dumps(ex, ensure_ascii=False)


def train_unsloth(args):
    print(f"[unsloth] Training {args.base} on {args.dataset}")
    try:
        from unsloth import FastLanguageModel
        from datasets import Dataset
        import torch
    except ImportError as e:
        print(f"Unsloth not installed: {e}")
        print('Install: pip install "unsloth[colab-new] @ git+https://github.com/unslothai/unsloth.git" trl transformers datasets accelerate peft bitsandbytes')
        sys.exit(1)

    # Import TRL (SFTConfig new API vs old TrainingArguments)
    try:
        from trl import SFTTrainer, SFTConfig
        use_sft_config = True
    except ImportError:
        from trl import SFTTrainer
        from transformers import TrainingArguments
        use_sft_config = False

    max_seq_length = args.max_seq_len
    # 8GB guard: cap seq len with warning
    if max_seq_length > 4096:
        print(f"[warn] max-seq-len {max_seq_length} likely OOMs on 8GB — capping to 4096")
        max_seq_length = 4096

    print(f"Loading base {args.base} (4-bit) ...")
    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name=args.base,
        max_seq_length=max_seq_length,
        dtype=None,
        load_in_4bit=True,
    )
    # Apply chat template if tokenizer lacks one (Qwen etc. usually have it)
    if getattr(tokenizer, "chat_template", None) is None:
        print("[info] tokenizer has no chat_template — using manual render")

    model = FastLanguageModel.get_peft_model(
        model,
        r=args.rank,
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj",
                        "gate_proj", "up_proj", "down_proj"],
        lora_alpha=args.alpha,
        lora_dropout=0,
        bias="none",
        use_gradient_checkpointing="unsloth",
        random_state=3407,
        use_rslora=False,
        loftq_config=None,
    )

    rows = load_jsonl(Path(args.dataset))
    if not rows:
        print(f"No valid rows in {args.dataset}", file=sys.stderr)
        sys.exit(1)
    print(f"Dataset: {len(rows)} samples from {args.dataset}")

    # Prefer tokenizer.apply_chat_template when messages present
    def render(ex):
        if isinstance(ex.get("messages"), list) and getattr(tokenizer, "chat_template", None):
            try:
                return {"text": tokenizer.apply_chat_template(
                    ex["messages"], tokenize=False, add_generation_prompt=False)}
            except Exception:
                pass
        return {"text": to_text_example(ex)}

    dataset = Dataset.from_list(rows).map(render, remove_columns=None)
    # Keep only text for training
    dataset = dataset.map(lambda ex: {"text": ex["text"]})

    # Train/val split (5% eval, min 1, max 200)
    n_eval = 0
    if len(dataset) >= 20:
        split = dataset.train_test_split(test_size=min(200, max(1, len(dataset) // 20)), seed=3407)
        train_ds, eval_ds = split["train"], split["test"]
        n_eval = len(eval_ds)
    else:
        train_ds, eval_ds = dataset, None
    print(f"Train: {len(train_ds)}  Eval: {n_eval}")

    bf16_ok = torch.cuda.is_available() and torch.cuda.is_bf16_supported()
    out = str(args.output)
    common = dict(
        per_device_train_batch_size=args.batch,
        gradient_accumulation_steps=args.grad_accum,
        warmup_steps=10,
        num_train_epochs=args.epochs,
        learning_rate=float(args.lr),
        fp16=not bf16_ok,
        bf16=bf16_ok,
        logging_steps=1,
        optim="adamw_8bit",
        weight_decay=0.01,
        lr_scheduler_type="linear",
        seed=3407,
        output_dir=out,
        save_steps=100,
        save_total_limit=2,
        report_to="none",
    )
    if eval_ds is not None:
        common.update(eval_strategy="steps", eval_steps=100, save_strategy="steps")

    if use_sft_config:
        sft_args = SFTConfig(
            dataset_text_field="text",
            max_seq_length=max_seq_length,
            packing=False,
            dataset_num_proc=2,
            **common,
        )
        trainer = SFTTrainer(
            model=model,
            processing_class=tokenizer,
            train_dataset=train_ds,
            eval_dataset=eval_ds,
            args=sft_args,
        )
    else:
        tr_args = TrainingArguments(**common)
        trainer = SFTTrainer(
            model=model,
            tokenizer=tokenizer,
            train_dataset=train_ds,
            eval_dataset=eval_ds,
            dataset_text_field="text",
            max_seq_length=max_seq_length,
            dataset_num_proc=2,
            args=tr_args,
        )
    trainer.train()
    model.save_pretrained(out)
    tokenizer.save_pretrained(out)
    print(f"Saved adapter to {out}")
    print("Next: local-ai finetune merge --base <base> --adapter <out> --out ./outputs/merged")


def train_mlx(args):
    """Apple Silicon: convert sharegpt JSONL -> MLX train/valid, then run mlx_lm.lora."""
    print(f"[mlx] Training on Apple Silicon — base {args.base}")
    try:
        import mlx  # noqa: F401
    except ImportError:
        print("MLX not installed: pip install mlx mlx-lm")
        print("Then re-run this command.")
        sys.exit(1)

    rows = load_jsonl(Path(args.dataset))
    if not rows:
        print(f"No valid rows in {args.dataset}", file=sys.stderr)
        sys.exit(1)

    # MLX expects {"train": [...], "valid": [...]} with messages or prompt/completion
    out_data = Path(args.output).parent / "mlx_data"
    out_data.mkdir(parents=True, exist_ok=True)
    n_val = min(100, max(1, len(rows) // 20)) if len(rows) >= 20 else 0
    train_rows = rows[: len(rows) - n_val] if n_val else rows
    valid_rows = rows[len(rows) - n_val:] if n_val else []

    def mlx_row(ex):
        if isinstance(ex.get("messages"), list):
            return {"messages": ex["messages"]}
        return {"prompt": ex.get("instruction", "Complete the code:"),
                "completion": ex.get("output") or ex.get("text", "")}

    (out_data / "train.jsonl").write_text(
        "\n".join(json.dumps(mlx_row(r), ensure_ascii=False) for r in train_rows), encoding="utf-8")
    if valid_rows:
        (out_data / "valid.jsonl").write_text(
            "\n".join(json.dumps(mlx_row(r), ensure_ascii=False) for r in valid_rows), encoding="utf-8")
    print(f"MLX data: {len(train_rows)} train / {len(valid_rows)} valid -> {out_data}")

    adapter_out = str(args.output)
    # iters heuristic: epochs * samples / batch, capped for laptop
    iters = max(200, min(5000, args.epochs * len(train_rows) // max(1, args.batch)))
    cmd = (f"python -m mlx_lm.lora --model {args.base} --train "
           f"--data {out_data} --adapter-path {adapter_out} "
           f"--num-layers 16 --batch-size {args.batch} --iters {iters}")
    print(f"Running: {cmd}")
    rc = os.system(cmd)
    if rc != 0:
        sys.exit(rc)
    print(f"MLX adapter -> {adapter_out}")


def train_torchtune(args):
    """PyTorch-native torchtune: generate a QLoRA recipe config and run `tune`."""
    print(f"[torchtune] base {args.base} dataset {args.dataset}")
    rows = load_jsonl(Path(args.dataset))
    print(f"Dataset: {len(rows)} samples")
    cfg_path = Path(args.output).parent / "torchtune_config.yaml"
    cfg_path.parent.mkdir(parents=True, exist_ok=True)
    cfg = f"""# Auto-generated by local-ai finetune (torchtune QLoRA)
model:
  _component_: torchtune.models.qwen2.qlora
  base_model: {args.base}
  lora_rank: {args.rank}
  lora_alpha: {args.alpha}
tokenizer:
  _component_: torchtune.models.qwen2.tokenizer
  path: {args.base}
dataset:
  _component_: torchtune.datasets.chat_dataset
  source: json
  data_files: [{args.dataset}]
  train_on_input: false
  max_seq_len: {args.max_seq_len}
checkpointer:
  _component_: torchtune.training.FullModelTorchTuneCheckpointer
  checkpoint_dir: {args.output}
  output_dir: {args.output}
  should_load_recipe_state: false
loss:
  _component_: torchtune.modules.loss.CEWithChunkedOutputLoss
optimizer:
  _component_: torch.optim.AdamW
  lr: {args.lr}
epochs: {args.epochs}
batch_size: {args.batch}
gradient_accumulation_steps: {args.grad_accum}
"""
    cfg_path.write_text(cfg, encoding="utf-8")
    print(f"Wrote torchtune config -> {cfg_path}")
    print("Run: tune run lora_finetune_single_device --config " + str(cfg_path))
    print("Docs: https://pytorch.org/torchtune/ (pip install torchtune)")
    rc = os.system(f"tune run lora_finetune_single_device --config {cfg_path}")
    if rc != 0:
        print("[info] `tune` CLI not available or failed — config kept for manual run.")
        print("Install: pip install torchtune torch torchao")


def train_axolotl(args):
    """Multi-GPU Axolotl: generate YAML and run `axolotl train`."""
    print(f"[axolotl] base {args.base} dataset {args.dataset}")
    cfg_path = Path(args.output).parent / "axolotl_config.yml"
    cfg_path.parent.mkdir(parents=True, exist_ok=True)
    cfg = f"""# Auto-generated by local-ai finetune (Axolotl QLoRA, 8GB-safe)
base_model: {args.base}
load_in_4bit: true
model_type: AutoModelForCausalLM
tokenizer_type: AutoTokenizer
datasets:
  - path: {args.dataset}
    type: sharegpt
    conversation: chatml
dataset_prepared_path: null
val_set_size: 0.05
output_dir: {args.output}
sequence_len: {args.max_seq_len}
sample_packing: false
pad_to_sequence_len: true
adapter: qlora
lora_r: {args.rank}
lora_alpha: {args.alpha}
lora_dropout: 0.0
lora_target_modules: [q_proj, k_proj, v_proj, o_proj, gate_proj, up_proj, down_proj]
gradient_accumulation_steps: {args.grad_accum}
micro_batch_size: {args.batch}
num_epochs: {args.epochs}
optimizer: adamw_8bit
lr_scheduler: linear
learning_rate: {args.lr}
train_on_inputs: false
bf16: auto
fp16: false
tf32: false
gradient_checkpointing: true
early_stopping_patience:
logging_steps: 1
save_steps: 100
save_total_limit: 2
"""
    cfg_path.write_text(cfg, encoding="utf-8")
    print(f"Wrote axolotl config -> {cfg_path}")
    rc = os.system(f"axolotl train {cfg_path}")
    if rc != 0:
        print("[info] `axolotl` CLI not available — config kept for manual run.")
        print("Install: pip install axolotl && axolotl train " + str(cfg_path))


def main():
    p = argparse.ArgumentParser(description="Local-AI finetune dispatcher")
    p.add_argument("--backend", default="unsloth",
                   choices=["unsloth", "mlx", "torchtune", "axolotl"])
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

    if not Path(args.dataset).exists():
        print(f"Dataset not found: {args.dataset}", file=sys.stderr)
        sys.exit(1)

    if args.backend == "unsloth":
        train_unsloth(args)
    elif args.backend == "mlx":
        train_mlx(args)
    elif args.backend == "torchtune":
        train_torchtune(args)
    elif args.backend == "axolotl":
        train_axolotl(args)
    else:
        print(f"Unknown backend: {args.backend}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
