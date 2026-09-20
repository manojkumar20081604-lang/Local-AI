#!/usr/bin/env python3
import argparse, sys
p = argparse.ArgumentParser()
p.add_argument("--model", required=True)
p.add_argument("--out", required=True)
p.add_argument("--quant", default="q4_k_m")
args = p.parse_args()
print(f"Quantizing {args.model} -> {args.out} ({args.quant})")
try:
    from unsloth import FastLanguageModel
    model, tokenizer = FastLanguageModel.from_pretrained(model_name=str(args.model), max_seq_length=4096, dtype=None, load_in_4bit=False)
    model.save_pretrained_gguf(str(args.out), tokenizer, quantization_method=args.quant)
    print(f"Saved GGUF to {args.out}")
except Exception as e:
    print(f"GGUF export via Unsloth failed: {e}")
    print("Alternative: use llama.cpp quantize")
    print("  git clone https://github.com/ggerganov/llama.cpp && cd llama.cpp && make")
    print("  ./llama-quantize ./outputs/merged/ggml-model-f16.gguf ./outputs/gguf/model-q4_k_m.gguf q4_k_m")
    sys.exit(1)
