#!/usr/bin/env python3
import argparse
p = argparse.ArgumentParser()
p.add_argument("--base", required=True)
p.add_argument("--adapter", required=True)
p.add_argument("--out", required=True)
args = p.parse_args()
print(f"Merging {args.adapter} into {args.base} -> {args.out}")
try:
    from unsloth import FastLanguageModel
    model, tok = FastLanguageModel.from_pretrained(model_name=args.base, max_seq_length=4096, dtype=None, load_in_4bit=False)
    model.load_adapter(args.adapter)
    model.save_pretrained_merged(args.out, tok, save_method="merged_16bit")
    print(f"Merged to {args.out}")
except ImportError:
    try:
        from peft import PeftModel
        from transformers import AutoModelForCausalLM, AutoTokenizer
        import torch
        base = AutoModelForCausalLM.from_pretrained(args.base, torch_dtype=torch.float16, device_map="auto")
        model = PeftModel.from_pretrained(base, args.adapter)
        merged = model.merge_and_unload()
        merged.save_pretrained(args.out)
        AutoTokenizer.from_pretrained(args.base).save_pretrained(args.out)
        print(f"Merged to {args.out} (peft)")
    except Exception as e:
        print(f"Merge failed: {e}")
        print("Install: pip install peft transformers accelerate")
        raise
