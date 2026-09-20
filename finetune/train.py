#!/usr/bin/env python3
"""
Minimal finetune dispatcher — supports unsloth/mlx/torchtune/axolotl backends.

NOTE: Finetuned model still goes through same verifier (cli/src/core/verifier.rs +
finetune/verify.py) — finetuning does NOT bypass grounding. All responses are
post-verified for hallucinations (invented files/symbols, invented/mentioned >0.3).
This is by design: finetuning improves style/domain, not factual grounding.
"""
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
        print("Install: pip install \"unsloth[colab-new] @ git+https://github.com/unslothai/unsloth.git\" trl transformers datasets accelerate")
        sys.exit(1)

def train_mlx(args):
    print(f"[mlx] Training on Apple Silicon — base {args.base}")
    print("MLX finetune requires: pip install mlx mlx-lm")
    print("Example: python -m mlx_lm.lora --model {} --train --data {} --adapter-path {}".format(args.base, args.dataset, args.output))
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
