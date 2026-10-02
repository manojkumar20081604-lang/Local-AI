#!/usr/bin/env python3
"""Merge LoRA adapter into base model (16-bit) for GGUF/vLLM export.

Tries Unsloth (fast) -> PEFT (generic). Validates inputs first.
Usage: python3 finetune/merge.py --base Qwen/Qwen2.5-7B-Instruct --adapter ./outputs/lora-adapter --out ./outputs/merged
"""
import argparse
import sys
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--base", required=True)
    p.add_argument("--adapter", required=True)
    p.add_argument("--out", required=True)
    args = p.parse_args()

    adapter = Path(args.adapter)
    if not adapter.exists():
        print(f"Adapter not found: {adapter}", file=sys.stderr)
        sys.exit(1)
    # Adapter must contain either adapter_model.safetensors/.bin or adapter_config.json
    has_weights = (adapter / "adapter_model.safetensors").exists() or (adapter / "adapter_model.bin").exists()
    has_cfg = (adapter / "adapter_config.json").exists()
    if not (has_weights or has_cfg):
        print(f"[warn] {adapter} doesn't look like a PEFT adapter (no adapter_model.* / adapter_config.json). Continuing anyway...")

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    # 1) Unsloth fast merge
    try:
        from unsloth import FastLanguageModel
        print(f"[unsloth] Merging {args.adapter} into {args.base} ...")
        model, tok = FastLanguageModel.from_pretrained(
            model_name=args.base, max_seq_length=4096, dtype=None, load_in_4bit=False)
        # Unsloth 2024+: load adapter then save merged
        try:
            from peft import PeftModel
            model = PeftModel.from_pretrained(model, str(adapter))
            merged = model.merge_and_unload()
            merged.save_pretrained(str(out))
            tok.save_pretrained(str(out))
        except Exception:
            # Fallback to Unsloth native merged save
            model.load_adapter(str(adapter))
            model.save_pretrained_merged(str(out), tok, save_method="merged_16bit")
        print(f"Merged to {out} (unsloth)")
        return
    except ImportError:
        print("[info] unsloth not installed, trying PEFT ...")
    except Exception as e:
        print(f"[warn] unsloth merge failed: {e} — trying PEFT ...")

    # 2) PEFT generic merge
    try:
        from peft import PeftModel
        from transformers import AutoModelForCausalLM, AutoTokenizer
        import torch
        dtype = torch.bfloat16 if torch.cuda.is_available() and torch.cuda.is_bf16_supported() else torch.float16
        print(f"[peft] Loading base {args.base} ({dtype}) ...")
        base = AutoModelForCausalLM.from_pretrained(args.base, torch_dtype=dtype, device_map="auto", trust_remote_code=True)
        model = PeftModel.from_pretrained(base, str(adapter))
        merged = model.merge_and_unload()
        merged.save_pretrained(str(out))
        AutoTokenizer.from_pretrained(args.base, trust_remote_code=True).save_pretrained(str(out))
        print(f"Merged to {out} (peft)")
    except Exception as e:
        print(f"Merge failed: {e}")
        print("Install: pip install peft transformers accelerate bitsandbytes")
        raise


if __name__ == "__main__":
    main()
