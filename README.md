# Local AI

A local AI-powered developer workspace for **local models + finetuning**. Now as a **cross-platform CLI** (Linux, macOS, Windows) — no GUI required.

Local AI helps developers understand, analyze, and modify projects using locally running AI models via LM Studio, with best-in-class finetuning (QLoRA) that exports directly to LM Studio.

> **GUI (Tauri) deprecated** — see `cli/` for the CLI. The frontend remains in `frontend/` for reference but CLI is the primary interface.

## Features

- **Plan / Build modes** — `plan` (read-only, can't build/write/exec via CLI, AI only plans) vs `build` (allow writes, default). Flag `--mode plan|build`, env `LOCAL_AI_MODE`, config `mode = "plan"`. Enforced in `core/config.rs:184` (`require_build_mode`). Plan still allows `exec`/`web_fetch` tools for reading.
- **Browser + Terminal access** — model has `exec(command)` (terminal: `ls -R`, `cat`, `find`, `grep -rn`, `head` — reads all necessary files) and `web_fetch(url)` (browser: `https://` via `reqwest`, 10s timeout) via `--tools`. Offline core, but on-demand web when you enable tools. See `cli/README.md: Browser + Terminal`.
- **Universal local models** — LM Studio (`:1234/v1`), Ollama (`:11434` native + `/v1` compat), llama.cpp (`:8080`), vLLM/Tabby/any OpenAI-compatible, auto-detect (Ollama → LM Studio → llama.cpp)
- **Anti-hallucination 5 layers** — deterministic inventory router (no LLM), hybrid retrieval (0.55 cosine + 0.35 keyword), strict/balanced/creative prompts (0.2/0.4/0.7 temp), tool calling (`read_project_file`), post-generation verifier (lexical 0.76 F1 + `groundrails`/`LettuceDetect` 0.82/0.689)
- Streaming AI responses + local model selection (unified `AIModel` with `provider` column)
- Persistent projects + chat history (`projects.json` per OS data dir)
- Hybrid retrieval (offline `fastembed` `bge-small-en-v1.5` 120MB 384 dim or `TfIdf` fallback, Ollama `nomic-embed-text` if up) — index `~/.cache/local-ai/<id>/index.json` chunk 1500/200
- Project Intelligence (intent detection, relevant-file ranking, inventory header)
- Real project-file reading + safe writing (path traversal protection) + exec (`sh -c`/`cmd /C`)
- AI project analysis with citations + `analyze --dry-run` verifier preview
- AI-generated file edits (`<CREATE_FILE>/<EDIT>/<DELETE>/<EXEC>`) with `SEARCH` verification
- Integrated `doctor` + `config show|set` + `index rebuild|status` + `verify` (lexical + `finetune/verify.py` hybrid)

## Quick Start (CLI)

```bash
# Build
cd cli && cargo build --release && ./target/release/local-ai --help

# Attach a project and chat (auto-detects LM Studio or Ollama)
local-ai project attach . --name MyApp
local-ai models list                     # auto: Ollama -> LM Studio -> llama.cpp
local-ai models list --provider ollama  # or --provider lmstudio --url http://localhost:1234/v1
local-ai doctor                          # health_check all providers
local-ai chat "explain this repo" --project MyApp --grounding strict --show-verifier
local-ai analyze "what is the architecture?" --project MyApp --dry-run --show-verifier

# Files & exec
local-ai files list --project MyApp
local-ai exec --project MyApp -- "npm test"

# Hybrid retrieval (offline, 8GB safe)
local-ai index rebuild --project MyApp   # fastembed or TfIdf, hybrid ranking
local-ai index status --project MyApp

# Finetune (RTX 5050 8GB: Unsloth QLoRA)
local-ai finetune status
local-ai finetune prepare --project ./my-app --out ./dataset.jsonl
local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --dry-run
```

See `cli/README.md` and `finetune/README.md` for full docs.

## Features (Detail)

- CLI for all OS (single Rust binary, cross-platform)
- Universal provider (`cli/src/core/provider/*` — `lmstudio.rs`, `ollama.rs`, `generic.rs`, trait `Provider`, `autodetect`, `config.toml`)
- Project Intelligence (hybrid `fastembed` + keyword, intent detection, `intelligence.rs:202`)
- Persistent projects (`projects.json` per OS data dir) + `index.json` cache per project
- Path traversal protection, safe file writes, undo logic, `SEARCH` verification
- Finetuning: Unsloth (2-5x faster, 5-6GB for 7B QLoRA on RTX 5050), MLX (Apple Silicon), Axolotl, torchtune — **finetuned model still goes through same verifier (does not bypass grounding)**
- Prepare → Train → Merge → Quantize (GGUF) → Load in LM Studio
- Legacy GUI: Tauri + React (in `frontend/` + `src-tauri/` — deprecated)

## Architecture

```text
Local AI
├── cli/                 # Rust CLI (clap, tokio, reqwest) — PRIMARY
│   ├── src/core/provider/ # lmstudio.rs (SSE), ollama.rs (NDJSON + /v1 compat), generic.rs, mod.rs (trait + autodetect)
│   │   ├── config.rs     # ~/.config/local-ai/config.toml (ProviderKind, Grounding, Embeddings)
│   │   ├── embeddings.rs # fastembed-rs 5.13 (bge-small 384 dim) + Ollama nomic-embed-text + TfIdf fallback
│   │   ├── index.rs      # chunk 1500/200, blake3, flat JSON cosine (Qdrant/iQDB deferred)
│   │   ├── intelligence.rs # detect_intent + rank_relevant_files + hybrid_rank (0.55 cosine + 0.35 keyword)
│   │   ├── verifier.rs   # groundrails lexical + try_verify_with_python (groundrails+LettuceDetect) + symbol/edit checks
│   │   └── tools.rs      # list_project_files, read_project_file, search_project (Ollama/LM Studio tools)
│   ├── src/commands/     # chat.rs (router+prompt+verifier+retry), models.rs, analyze.rs, doctor.rs, config.rs, index.rs
│   └── tests/            # hallucination.rs (groundrails/LettuceDetect scorer), providers.rs (mock SSE/NDJSON), rag.rs (fastembed/TfIdf)
├── finetune/            # Python QLoRA pipelines (Unsloth/MLX/axolotl/torchtune)
│   ├── train.py         # dispatched by `local-ai finetune train` — finetuned model still verified (no bypass)
│   ├── verify.py        # groundrails + LettuceDetect v2-mmbert-base + ragground + groundlens hybrid (lexical fallback)
│   ├── merge.py + quantize.py  # GGUF for LM Studio
│   └── requirements.txt # datasets, transformers, peft, groundrails, lettucedetect, ragground, groundlens
├── frontend/            # (legacy) React + Vite GUI — deprecated
└── src-tauri/           # (legacy) Tauri backend — logic ported to cli/src/core
```
