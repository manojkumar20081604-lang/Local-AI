# Local AI CLI

Cross-platform CLI for local AI workspace + finetuning. Replaces the Tauri GUI with a single binary that works on **Linux, macOS, Windows**.

## Install

### From source (all OS - requires Rust >=1.77)

```bash
git clone https://github.com/local-ai/local-ai
cd local-ai/cli
cargo build --release
# binary at ./target/release/local-ai
sudo cp target/release/local-ai /usr/local/bin/
# or cargo install --path .
```

### Prebuilt binaries (GitHub Releases)

```bash
# Linux x86_64
curl -L https://github.com/local-ai/local-ai/releases/latest/download/local-ai-linux-x86_64 -o local-ai && chmod +x local-ai
# macOS Intel
curl -L https://github.com/local-ai/local-ai/releases/latest/download/local-ai-darwin-x86_64 -o local-ai && chmod +x local-ai
# macOS Apple Silicon
curl -L https://github.com/local-ai/local-ai/releases/latest/download/local-ai-darwin-arm64 -o local-ai && chmod +x local-ai
# Windows
# Download local-ai-windows-x86_64.exe from Releases
```

## Modes — Plan / Build

* **plan** — read-only, no writes/exec/build. AI only outputs PLAN (no `<CREATE_FILE>/<EDIT>` blocks). Use for safe exploration.
* **build** — allow writes/exec/finetune (default). AI can propose and apply file ops.

```bash
local-ai --mode plan chat "create a feature" --project MyApp   # only plans, not writes
local-ai --mode build chat "implement it" --project MyApp      # can build
local-ai --mode plan files write src/new.rs --content "..."    # ❌ blocked: Plan mode
local-ai --mode plan exec --project MyApp -- "npm test"        # ❌ blocked
LOCAL_AI_MODE=plan local-ai doctor                             # env override
local-ai config set mode plan   # persist to ~/.config/local-ai/config.toml
local-ai config set mode build  # back to build
local-ai config show            # shows mode + provider + grounding
```

When plan mode is active, `doctor` shows `⚠ Plan mode ACTIVE`, `chat` system prompt says `PLAN MODE ACTIVE`, and `files write/delete`, `exec` (CLI), `finetune train/merge/quantize/pipeline` are blocked with message `Switch to build mode`. Read via `files list/read` and terminal `exec` tool (`ls/cat/find/grep`) + browser `web_fetch` are still allowed for reading.

## Browser + Terminal — Read All Necessary Files

Model now has **browser + terminal access** (opt-in via `--tools`) to read all necessary files — as requested:

* **Terminal `exec`** — `exec(command)` tool runs `sh -c` in project folder. Examples the model can call:
  ```bash
  ls -R | head -100          # list all files
  cat src/main.rs            # read file
  find . -name "*.rs" | xargs wc -l   # count lines
  grep -rn "getUser" src/ --include="*.rs"  # search symbols
  head -n 50 Cargo.toml       # preview
  ```
  In plan mode, only read-only commands (`ls/cat/find/grep/head/wc`) are allowed — write/build (`rm/cargo build/npm build/mkdir`) are blocked. In build mode, all commands allowed.

* **Browser `web_fetch`** — `web_fetch(url, max_chars=4000)` tool fetches `https://` URLs via `reqwest` (10s timeout, `User-Agent: Local-AI/0.1`). Returns text (truncated). Examples:
  ```bash
  web_fetch(url="https://doc.rust-lang.org/book/ch01-00-getting-started.html")
  web_fetch(url="https://crates.io/crates/fastembed")
  web_fetch(url="https://docs.rs/tokio/latest/tokio/")
  ```
  Allowed in both plan/build (read-only). Use `local-ai chat "..." --tools --project MyApp` to let the model call `exec`/`web_fetch` instead of hallucinating.

No default web search (offline core), but `web_fetch` gives on-demand browser access when you add `--tools`. For CLI manual fetch, use `local-ai exec -- "curl https://example.com"` or `web_fetch` via tool.

## Usage

```bash
local-ai --help  # shows --mode plan|build, --provider auto|ollama|lmstudio, --grounding strict|balanced|creative

# Projects — uses same storage as GUI: ~/.local/share/com.localai.app/projects.json (Linux)
local-ai project list
local-ai project attach . --name MyApp
local-ai project attach /path/to/folder --name MyApp
local-ai project info

# Files
local-ai files list --project MyApp
local-ai files list --project /path/to/folder
local-ai files read src/main.rs --project MyApp
local-ai files write src/new.rs --content "fn main(){}" --project MyApp
local-ai files delete src/old.rs --project MyApp --force

# Execute commands in project
local-ai exec --project MyApp -- "npm test"
local-ai exec -- "cargo build"  # uses current dir as project
local-ai exec --project MyApp -- "ls -la"

# Models — universal provider (LM Studio + Ollama + llama.cpp + any OpenAI-compatible)
local-ai models list                              # auto-detect: tries Ollama :11434 -> LM Studio :1234/v1 -> llama.cpp :8080
local-ai models list --provider ollama            # force Ollama native (GET /api/tags)
local-ai models list --provider lmstudio --url http://localhost:1234/v1
local-ai doctor                                   # health_check all providers + embeddings + grounding config
local-ai config show                              # dump merged config + autodetect
local-ai config set provider.ollama.url http://localhost:11434

# Chat (streaming, project-aware, grounded, saves to projects.json)
local-ai chat "explain the auth flow" --project MyApp --model qwen/qwen3.5-9b
local-ai chat --project MyApp                     # interactive REPL (/help, /clear, /history, /model, /grounding)
local-ai chat "what is the architecture?" --show-context --show-verifier --grounding strict
local-ai chat "fix the bug" --no-context --no-save --grounding creative
local-ai chat "explain src/main.rs" --project MyApp --tools   # tool calling: model must call read_project_file() to avoid hallucination

# Analyze (intent detection + hybrid retrieval + verifier)
local-ai analyze "explain the architecture" --project MyApp
local-ai analyze "where is the database code?" --project MyApp --dry-run  # only show relevant files + verifier preview, no LLM call
local-ai analyze "create src/services/authService.ts" --project MyApp --dry-run --show-verifier  # would-reject invented files
# Verifier always runs post-generation; strict mode exits 2 on hallucination (for CI)

# Index (hybrid retrieval — local embeddings, no cloud)
local-ai index rebuild --project MyApp           # build ~/.cache/local-ai/<id>/index.json (chunk 1500/200, bge-small-en-v1.5 or TfIdf)
local-ai index status --project MyApp
local-ai index rebuild --project ./my-app --embeddings fastembed  # force fastembed (120MB ONNX, 384 dim, ~180ms/query CPU)

# Finetune — best for local models
local-ai finetune status  # shows GPU, OS, recommended backend
local-ai finetune prepare --project ./my-app --out ./dataset.jsonl --format sharegpt
local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --dry-run
local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --output ./outputs/lora-adapter --rank 16 --alpha 32
local-ai finetune merge --base Qwen/Qwen2.5-7B-Instruct --adapter ./outputs/lora-adapter --out ./outputs/merged
local-ai finetune quantize --model ./outputs/merged --out ./outputs/gguf --quant q4_k_m
local-ai finetune pipeline --project ./my-app --base Qwen/Qwen2.5-7B-Instruct --dry-run  # full pipeline
```

## Project Context

Same as GUI's `Project Intelligence`:
- Detects intent (explain/debug/edit/find/architecture)
- Ranks relevant files (ignores node_modules/.git/target/dist)
- Injects top 6 files (8000 chars each) into system prompt
- Handles `<CREATE_FILE>`, `<EDIT>`, `<DELETE>`, `<EXEC>` blocks

Storage: `projects.json` in OS data dir:
- Linux: `~/.local/share/com.localai.app/projects.json`
- macOS: `~/Library/Application Support/com.localai.app/projects.json`
- Windows: `%APPDATA%\com.localai.app\projects.json`

## Providers — Universal Local Models

One CLI, any local backend. Auto-detect order: **Ollama native (:11434) → LM Studio (:1234/v1) → llama.cpp (:8080) → generic (user --url)**. 5s connect timeout.

| Provider | Default URL | List | Chat Stream | Notes |
|---|---|---|---|---|
| **LM Studio** | `http://localhost:1234/v1` | `GET /v1/models` | `POST /v1/chat/completions` SSE | Keep `--lm-studio-url` as alias (deprecated) |
| **Ollama native** | `http://localhost:11434` | `GET /api/tags` | `POST /api/chat` NDJSON | Auto-translates to unified `AIModel {id, owned_by="ollama"}` |
| **Ollama compat** | `http://localhost:11434/v1` | `GET /v1/models` | `POST /v1/chat/completions` | Fallback if native fails |
| **llama.cpp** | `http://localhost:8080` | `GET /v1/models` or `/props` | `POST /v1/chat/completions` | For GGUF directly |
| **Generic OpenAI** | `--url` | `GET /v1/models` | `POST /v1/chat/completions` | vLLM, Tabby, GPT4All, etc. |

Config `~/.config/local-ai/config.toml` (Linux/macOS/Windows `%APPDATA%`):
```toml
[provider]
active = "auto" # auto | lmstudio | ollama | llamacpp | generic
[providers.lmstudio]
url = "http://localhost:1234/v1"
[providers.ollama]
url = "http://localhost:11434"
[grounding]
mode = "balanced" # strict | balanced | creative (strict=0.2 temp, balanced=0.4, creative=0.7)
[embeddings]
provider = "tfidf" # tfidf (default offline) | fastembed | ollama
model = "BAAI/bge-small-en-v1.5" # 384 dim, 120MB ONNX quantized
```

CLI flags (global):
```bash
local-ai --provider auto|lmstudio|ollama|generic --url http://localhost:11434 --grounding strict chat "..."
local-ai --provider ollama models list
local-ai doctor  # shows which provider is up + embeddings backend + grounding
```

Env overrides (backwards compat): `LOCAL_AI_URL`, `LOCAL_AI_LM_STUDIO_URL`, `LOCAL_AI_PROVIDER`, `LOCAL_AI_GROUNDING`.

## Grounding & Anti-Hallucination (5 Layers)

No single prompt fixes 0.7 temperature hallucinations. Defense in depth:

0. **Deterministic Router** — inventory / `what is project name?` answered directly from `files list` without LLM (100% precision). Ported from `frontend/App.tsx:1258`.
1. **Grounded Retrieval** — hybrid `0.55 cosine + 0.35 keyword + 0.10` (threshold 0.25, top 6). Chunk 1500/200, `blake3` hash, cache `~/.cache/local-ai/<id>/index.json`. Inventory header injected BEFORE file contents so model sees authoritative list.
2. **Constrained Prompt** — `strict` (must cite `[src/main.rs:12]`, else "Not in project context"), `balanced` (allows Suggestions), `creative` (allows Hypothetical:). Temperature per mode 0.2/0.4/0.7.
3. **Structured Tools** — `list_project_files`, `read_project_file(path)`, `search_project(query)`, `exec(command)`, `web_fetch(url)` via Ollama/LM Studio tool calling. Fallback to `<EDIT>` blocks if not supported. `exec` gives terminal access to read all files (`ls/cat/find/grep`), `web_fetch` gives browser access.
4. **Post-Generation Verifier** — `core/verifier.rs` lexical (`~165ms/claim`, 0.76 F1) + optional Python `finetune/verify.py` hybrid (`groundrails` 0.82 F1 + `LettuceDetect v2-mmbert-base` 150M 4K ctx 0.642 F1 / qwen-2b 0.689). Extracts `File:` mentions, checks `file_set`, scores `invented/mentioned`. Strict: any invented → hallucinated; Balanced: `>0.3` or `>2`; Creative: `>0.6`. Auto-retry once with temp 0.2 on strict hallucination, else fallback to `Not in context` + `files list` suggestion. Also verifies `SEARCH` in `<EDIT>` blocks and symbol existence via grep.
5. **UX** — `--show-context`, `--show-verifier` (per-claim `grounded/score/support`), `--show-verifier --inject-citations` (`ragground`-style `[1]`), `analyze --dry-run` shows would-reject list, `doctor` shows grounding+embeddings.

Example:
```bash
local-ai chat "create src/services/authService.ts" --project MyApp --grounding strict --show-verifier
# ⚠️  Unverified paths (not in project): src/services/authService.ts — removed from answer.
# Retrying with stricter context...
# Assistant (retry): Not in project context — file `src/services/authService.ts` not found ...
```

## Retrieval — Hybrid Embeddings (Offline, 8GB safe)

- **Preferred if Ollama up**: `nomic-embed-text` (768 dim) via `POST /api/embeddings`
- **Default offline**: `TfIdf` (384 dim, no download) or `fastembed-rs` `BAAI/bge-small-en-v1.5` (384 dim, 120MB ONNX, ~180ms/query CPU, `ort` + `tokenizers`). Model auto-downloads via `hf-hub` on first `index rebuild` if cached; else fallback to TfIdf to avoid hang.
- **Index**: flat JSON brute-force `cosine` (<10k files <50ms). >10k: defer to `qdrant_edge`/`iQDB` (not yet, spec).
- **Chunk**: 1500/200 with newline snap, file hash `blake3`, mtime invalidation.
- **Vector store**: `~/.cache/local-ai/<project-id>/index.json` — rebuild on `mtime` change.

Commands:
```bash
local-ai index rebuild --project MyApp
local-ai index status --project MyApp  # shows stale, embedder, dims, age
```

## Verifier — Direct Usage

```bash
# CLI verifier (post-generation, same as chat.rs)
local-ai chat "explain auth" --project MyApp --show-verifier --grounding strict  # prints [verifier] invented/score/hallucinated + symbol checks
local-ai analyze "where is DB code?" --project MyApp --show-verifier --dry-run
# Python hybrid (requires pip install -r finetune/requirements.txt)
python3 finetune/verify.py --answer "File: src/foo.ts does X" --sources src/main.rs src/lib.rs --mode strict --json
python3 finetune/verify.py --answer-file answer.txt --sources-dir ./src --mode balanced --show-support --inject-citations
echo "invented src/foo.ts" | python3 finetune/verify.py --answer-stdin --sources cli/src/main.rs --json  # exit 2 on strict hallucination
```

## Finetune Backends

Auto-detected by `finetune status`:

- **Unsloth** (NVIDIA Linux/Windows, RTX 5050 8GB): 2-5x faster, 5-6GB for 7B QLoRA. `pip install "unsloth[colab-new] @ git+https://github.com/unslothai/unsloth.git"`
- **MLX** (macOS Apple Silicon): `pip install mlx mlx-lm`
- **torchtune** (fallback, CUDA+MPS): `pip install torchtune`
- **Axolotl** (multi-GPU): `pip install axolotl`

See `../finetune/README.md` for VRAM guide and dataset details.

## Development

```bash
cargo run -- --help
cargo run -- project list
cargo run -- finetune status
cargo build --release  # cross-platform: same command on all OS
```

## Path Safety

Reuses Tauri's path traversal protection: rejects absolute paths, `..`, and canonicalizes to ensure writes stay inside project folder.

## All OS Notes

- **Linux**: `sh -c` for exec, tested on Arch/Zen kernel
- **macOS**: `sh -c` for exec, MPS backend for finetune
- **Windows**: `cmd /C` for exec, paths use `/` internally, normalized via `MAIN_SEPARATOR`
