# Finetune — Local AI

Cross-platform fine-tuning for local models. Dispatched by `local-ai finetune` CLI.

## Quick start (RTX 5050 8GB, Linux)

```bash
# 1. Prepare dataset from your project
local-ai finetune prepare --project ./my-app --out ./dataset.jsonl --format sharegpt

# 2. Check environment
local-ai finetune status

# 3. Train (Unsloth QLoRA 4-bit — recommended for 8GB)
local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --output ./outputs/lora-adapter

# Or dry-run first:
local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --dry-run

# 4. Merge adapter → full model
local-ai finetune merge --base Qwen/Qwen2.5-7B-Instruct --adapter ./outputs/lora-adapter --out ./outputs/merged

# 5. Quantize → GGUF for LM Studio
local-ai finetune quantize --model ./outputs/merged --out ./outputs/gguf --quant q4_k_m

# 6. Load GGUF in LM Studio: My Models → Add Model → select outputs/gguf/*.gguf
```

## Backends auto-selected by CLI

| OS / GPU | Backend | VRAM 7B QLoRA | Install |
|---|---|---|---|
| **Linux/Windows + NVIDIA (RTX 5050)** | **Unsloth** | **5-6GB** | `pip install "unsloth[colab-new] @ git+https://github.com/unslothai/unsloth.git"` |
| macOS Apple Silicon | MLX | unified mem 16GB+ | `pip install mlx mlx-lm` |
| macOS / Linux fallback | torchtune | 10GB | `pip install torchtune` |
| Multi-GPU / complex pipelines | Axolotl | 16GB | `pip install axolotl` |

Override: `local-ai finetune train --backend mlx --base ...`

## Manual Python usage

```bash
python3 finetune/train.py --backend unsloth --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --output ./outputs/lora-adapter --rank 16 --alpha 32 --max-seq-len 4096 --epochs 3 --lr 0.0002 --batch 1 --grad-accum 4
python3 finetune/merge.py --base Qwen/Qwen2.5-7B-Instruct --adapter ./outputs/lora-adapter --out ./outputs/merged
python3 finetune/quantize.py --model ./outputs/merged --out ./outputs/gguf --quant q4_k_m
```

## Dataset formats

- `sharegpt` / `chatml`: `{"messages": [{"role":"system","content":...},{"role":"user","content":...},{"role":"assistant","content":...}]}` — best for chat models
- `alpaca`: `{"instruction":..., "input":..., "output":...}`

`prepare` creates one sample per file (up to --max-chars). Curate afterwards if needed.

## VRAM guide (QLoRA 4-bit, rank 16)

- 7B: 5-6GB (Unsloth) / 10GB (vanilla) — fits RTX 5050 8GB ✅
- 9B (Qwen3.5-9b): ~6-7GB Unsloth — fits 8GB with ctx 2048 ✅
- 13B: ~8GB Unsloth — tight on 8GB, use ctx 2048 + batch 1
- 70B: needs 24GB+ or multi-GPU with Axolotl

## Verification (Anti-Hallucination — finetuned model still verified)

Finetuning does **not** bypass grounding. Every response still goes through `cli/src/core/verifier.rs` (lexical + optional `finetune/verify.py` hybrid). Install verifier deps:

```bash
pip install -r finetune/requirements.txt  # includes groundrails, lettucedetect, ragground, groundlens
# or minimal lexical only (no ML): already in Rust — no Python needed
```

Use directly (CI gate like `jay-tank/nogrounds` NG001):

```bash
# Strict: invented file -> exit 2 (hallucinated)
python3 finetune/verify.py --answer "File: src/foo.ts does X" --sources src/main.rs src/lib.rs --mode strict --json
# Hybrid hybrid-lexical+lettucedetect for code hallucinations (`invented getUser()`)
python3 finetune/verify.py --answer-file answer.txt --sources-dir ./src --mode balanced --show-support --inject-citations
echo "invented src/foo.ts" | python3 finetune/verify.py --answer-stdin --sources cli/src/main.rs --json
# Lexical fallback (groundrails 0.76 F1, ~165ms/claim; semantic 0.82 F1 with --semantic bge-m3)
python3 finetune/verify.py --answer "..." --sources ./src --backend lexical --json
# LettuceDetect v2-mmbert-base (150M, 4K ctx, 0.642 F1 / qwen-2b 0.689) for code spans
python3 finetune/verify.py --answer "..." --sources ./src --backend lettucedetect --json
# ragground cited answer (<0.3ms ONNX, injects [1])
python3 finetune/verify.py --answer "..." --sources ./src --inject-citations --cited-answer
```

In Rust: `local-ai chat "..." --show-verifier --grounding strict` prints `mentioned/invented/score/hallucinated + symbol_checks + cited preview`, auto-retries once with temp 0.2 on strict hallucination, else fallback `Not in project context`.

`train.py` note: finetuned adapter still injected via same prompt + verifier — finetuning improves style/domain, not factual grounding. Always run `local-ai analyze --dry-run` + verifier before trusting edits.

## Troubleshooting

- Blackwell RTX 5050 needs `torch>=2.3 + CUDA 12.4+` and `unsloth` latest. If install fails, try `pip install --upgrade triton`.
- OOM: reduce `--max-seq-len 2048`, `--rank 8`, `--batch 1 --grad-accum 8`
- GGUF: requires `llama.cpp` or `unsloth` GGUF export. LM Studio can also import merged HF model directly (safetensors) without GGUF.
- Verifier: if `lettucedetect` not installed, `finetune/verify.py` falls back to pure Rust lexical (0.76 F1) — no error, just `backend: lexical`. For full ML: `pip install lettucedetect groundrails ragground "groundlens[encoder]" sentence-transformers`.
