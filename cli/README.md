# Local AI CLI

Cross-platform CLI for local AI workspace + finetuning. Replaces the Tauri GUI with a single binary that works on **Linux, macOS, Windows**.

## Install

### One command (recommended — `local-ai` works from any directory)

```bash
git clone https://github.com/manojkumar20081604-lang/Local-AI
cd Local-AI
./install.sh              # → ~/.local/bin/local-ai (no sudo)
./install.sh --system     # → /usr/local/bin/local-ai (needs sudo)
./install.sh --uninstall  # remove it again
```

If `~/.local/bin` is not on your PATH, add it once and restart the terminal:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
# fish: fish_add_path ~/.local/bin
```

### From source with cargo (same result)

```bash
cargo install --path cli            # installs `local-ai` to ~/.cargo/bin
# or build manually:
cd cli && cargo build --release     # binary at ./target/release/local-ai
```

Requires Rust >= 1.77 (https://rustup.rs).

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

# First run — provider → connection → model (saved globally, never in projects)
local-ai init                                   # interactive wizard (also runs on first bare launch)
local-ai                                        # bare: asks provider each launch until you pin one…
local-ai provider select --name ollama          # …pin it, and bare launches go silent ✓✓✓
local-ai init --provider ollama --model qwen3-coder  # non-interactive (scripts)
local-ai provider list                          # endpoints + reachability (* = active)
local-ai provider select --name ollama          # switch provider (re-checks saved model)
local-ai provider test --name lmstudio          # connectivity + model list
local-ai models select --id qwen2.5:1.5b        # pin the default model (flags still win)
local-ai config set model qwen2.5:1.5b          # same thing, script-friendly
local-ai doctor                                 # 9 checks: install/PATH/config/provider/model/project/git/terminal/tools

# Models — universal provider (LM Studio + Ollama + llama.cpp + any OpenAI-compatible)
local-ai models list                              # auto-detect: tries Ollama :11434 -> LM Studio :1234/v1 -> llama.cpp :8080
local-ai models list --provider ollama            # force Ollama native (GET /api/tags)
local-ai models list --provider lmstudio --url http://localhost:1234/v1
local-ai config show                              # dump merged config + autodetect
local-ai config set provider.ollama.url http://localhost:11434

# Chat (streaming, project-aware, grounded, saves to projects.json)
local-ai                                        # bare: first-run setup (once) → summary → TUI
local-ai "fix the PDF crash"                    # bare: agent loop in current dir (approval-gated)
local-ai tui --project MyApp                    # fullscreen anime command center (panels, diffs, approvals)
local-ai tui --no-animation --ascii --theme matrix  # calm/portable mode; custom ~/.config/local-ai/theme.json
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

# Git intelligence (read-only — safe in plan mode)
local-ai git status --project MyApp
local-ai git diff --project MyApp [--staged] [--file src/x.rs]
local-ai git log --project MyApp -n 20
local-ai git blame --project MyApp --file src/x.rs --lines 10,40
local-ai git branches --project MyApp
local-ai git commit --project MyApp --dry-run   # changeset preview (no commit)
local-ai git commit --project MyApp --yes       # build-gated, approval-gated, optional --test-cmd gate
local-ai git rollback --project MyApp --yes     # restore newest local-ai checkpoint (stash pop; build-gated)
# Checkpoints: agent/debug runs stash the dirty tree first (tracked + untracked);
# rollback restores it. No repo or clean tree → checkpoint skipped with a notice.

# Plan Mode 2.0 — approvable execution graphs (read-only)
local-ai plan "add JWT auth" --project MyApp            # human view + saves to cache
local-ai plan "fix crash" --project MyApp --json        # machine-readable graph
local-ai exec-plan ./plans/add-jwt-auth-<ts>.json --dry-run --project MyApp
local-ai exec-plan ./plans/add-jwt-auth-<ts>.json --yes --project MyApp  # build-gated

# Self-debugging loop (build-gated; --dry-run previews)
local-ai debug "fix failing tests" --project MyApp --max-attempts 3
local-ai debug --project MyApp --test-cmd "npm test" --yes
# Every proposed edit shows a unified-diff preview, then asks:
# Allow once / Allow for session / Reject (--yes skips; session never persists)

# Agent system — orchestrated specialists (foreground, bounded, no daemon)
local-ai agent run "add auth" --project MyApp --max-steps 20 --approve dangerous
local-ai agent run "fix failing tests" --project MyApp --dry-run   # preview only, plan-mode safe
local-ai agent run "fix tests" --project MyApp --yes --test-cmd "cargo test"  # skip approvals
# Roles: planner (1.4 graph) → researcher (read-only) → coder (SEARCH-verified) → tester/debugger (1.2 loop) → reviewer (verifier + secrets/traversal)
# Safety: budget (steps/tools/wall-time) + kill-switch (rm -rf /, mkfs, curl POST exfil blocked unless --approve dangerous)
# Trace: ~/.cache/local-ai/<id>/missions/<ts>/trace.jsonl (every agent I/O, feeds Phase 4/5)

# TUI — anime command center (new presentation layer over the same engine)
# Header HUD (model/provider/branch/progress) + state-driven anime sidekick +
# streaming chat + live plan/tasks + project tree + tool feed + test dashboard.
# Approvals arrive as modals ([a] once [s] session [r] reject) with the same
# diff preview; routes back into the agent loop via oneshot (fail-closed).
# Keys: Enter send · ↑↓ history/tasks · Tab panels · PgUp/PgDn scroll ·
# Ctrl+P palette (/help /model /theme /ascii /animation /clear /quit) ·
# Ctrl+C cancel run · Ctrl+D quit. Layout collapses 140→80→<80 cols.
# Needs a terminal; scripts/CI keep the plain subcommands (+ global --plain).

# Index (hybrid retrieval — local embeddings, no cloud)
local-ai index rebuild --project MyApp           # build ~/.cache/local-ai/<id>/index.json (chunk 1500/200, bge-small-en-v1.5 or TfIdf)
local-ai index status --project MyApp            # now also shows symbols=N files/M edges
local-ai index rebuild --project ./my-app --embeddings fastembed  # force fastembed (120MB ONNX, 384 dim, ~180ms/query CPU)
local-ai index rebuild --project MyApp --symbols=false  # skip code-graph extraction (default on)

# Memory — three tiers (project|user|task; show is read-only, set/forget need build)
local-ai memory show --scope project --project MyApp
local-ai memory set --scope project --project MyApp --content "uses React+Rust+Postgres; auth in src/auth.rs"
local-ai memory set --scope user --content "prefers TypeScript, short answers, Linux"
local-ai memory forget --scope project --project MyApp --filter "old stack"
# Auto-journalled after every `git commit` (files + stack, secrets redacted); injected into chat/analyze
# Project conventions: check a LOCAL-AI.md into the repo — it outranks stored
# memory in every prompt (chat/analyze/agent/debug), travels with the team,
# and is secrets-redacted + capped at 4000 chars. Example:
#   Use TypeScript. / Do not modify generated files. / Run npm test after changes.

# Code graph — symbols + imports (read-only, plan-mode safe)
local-ai graph --project MyApp --file src/auth.rs          # Imports / Imported-by / Functions / Tests / Recent changes
local-ai graph --project MyApp --query "where is auth handled?"  # symbol resolution, no LLM

# Chat routing — cheapest capable model (logs choice with --show-context)
local-ai chat "fix the login bug" --project MyApp --route   # coding → 7-15B model
local-ai chat "explain the architecture" --project MyApp --route  # reason → largest model
# Override per class in ~/.config/local-ai/config.toml: [router] coding = "qwen2.5-14b"

# Missions — persisted orchestrator state (same engine as `agent run`, kill-safe)
local-ai mission create "add stripe checkout" --project MyApp
local-ai mission list --project MyApp
local-ai mission show 24                    # TASKS/CHANGES/TESTS/LOGS from missions.json + trace.jsonl
local-ai mission show 24 --watch            # stream step updates until done
local-ai mission resume 24 --yes            # run remaining steps (persists per step — kill and resume)
local-ai mission cancel 24

# MCP tool servers — external tools alongside built-ins (JSON-RPC over stdio)
local-ai chat "list the docs" --project MyApp --tools --mcp filesystem
local-ai chat "summarize issues" --project MyApp --tools --mcp filesystem --mcp github
# Reference servers in mcp/ (python3, stdlib only, read-only first): filesystem, github
# Security: MCP writes/exec gated by plan/build + approval; env-exfil args need --approve dangerous

# Browse — web research with cited code blocks (never silent: every URL logged)
local-ai browse "stripe checkout api" --max-pages 5
local-ai browse "tokio spawn" --seed-url https://docs.rs/tokio --max-pages 3 --show-context

# Finetune — best for local models
local-ai finetune status  # shows GPU, OS, recommended backend
local-ai finetune prepare --project ./my-app --out ./dataset.jsonl --format sharegpt
local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --dry-run
local-ai finetune train --base Qwen/Qwen2.5-7B-Instruct --dataset ./dataset.jsonl --output ./outputs/lora-adapter --rank 16 --alpha 32
local-ai finetune merge --base Qwen/Qwen2.5-7B-Instruct --adapter ./outputs/lora-adapter --out ./outputs/merged
local-ai finetune quantize --model ./outputs/merged --out ./outputs/gguf --quant q4_k_m
local-ai finetune pipeline --project ./my-app --base Qwen/Qwen2.5-7B-Instruct --dry-run  # full pipeline
local-ai finetune eval --base bench/results/base.json --adapter bench/results/adapter.json  # offline gate (blocks on regression)
local-ai finetune eval --base qwen2.5:7b --adapter my-adapter --bench bench/prompts.jsonl --project MyApp  # live: streams both, saves runs, gates deploy

# Model Lab — benchmark studio (TPS, TTFT, pass@1 on fixture tasks)
local-ai bench run --model qwen2.5:9b --suite coding --project MyApp   # streams bench/prompts.jsonl, saves bench/results/<model>-<ts>.json
local-ai bench run --model X --prompt-set bench/prompts.jsonl --out ./bench/results
local-ai bench compare bench/results/a.json bench/results/b.json       # before/after table + GATE verdict

# Dataset feedback loop — approved interactions → QLoRA JSONL (reuses `finetune prepare` format)
local-ai dataset collect --project MyApp --from missions --only-approved --out ./dataset.jsonl
local-ai dataset collect --project MyApp --from all --format alpaca --out ./dataset.jsonl  # then: finetune train --dataset ./dataset.jsonl
# Finetuning never bypasses grounding: --only-approved keeps verifier-clean samples, verifier still runs post-deploy.

# Metrics + self-improvement proposals (plan-graphs, never silent self-mod)
local-ai metrics --project MyApp               # retrieval hit-rate, verifier reject-rate, test pass-rate (from traces + index)
local-ai metrics --project MyApp --json        # machine-readable for scripts
local-ai propose --project MyApp               # e.g. "retrieval failed 18% — suggest symbol retrieval (+12%)" with [Inspect] [Apply] [Reject]
local-ai propose --project MyApp --apply rebuild-index-symbols --yes  # saves plan to cache, prints exec-plan command
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
