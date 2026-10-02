# Local-AI — a local coding agent for your terminal

![Linux](https://img.shields.io/badge/OS-linux%20%7C%20macOS%20%7C%20Windows-blue)
![Rust](https://img.shields.io/badge/Rust-1.77%2B-orange)
![License](https://img.shields.io/badge/license-MIT-green)
![Local](https://img.shields.io/badge/inference-100%25%20local-brightgreen)

**Claude-Code-style terminal coding agent powered entirely by local models** — from small 7B models to 120B-class models. It understands your codebase, plans tasks, edits files, runs tests, verifies changes, and commits — all with local LLMs (LM Studio / Ollama / llama.cpp). Nothing ever leaves your machine.

> Legacy Tauri GUI is deprecated — the CLI (`cli/`) is the product. `frontend/` + `src-tauri/` remain for reference only.

## The 60-second demo

```text
$ cd my-project && local-ai "fix the PDF upload crash"

🔍 Inspecting project — 85 files
🧠 Ranked src/upload.py, tests/test_upload.py as relevant
📋 Plan: reproduce → isolate → fix → re-test → review   [Approve ✓]
✏️ Editing src/upload.py (+7 −7, diff preview shown)
🧪 pytest → ❌ 1 failed (UnicodeDecodeError: 'utf-8' … byte 0xe2)
🔧 Anchor-verified fix applied → 🧪 3/3 passed ✅
🔐 Verifier: 0 invented files → ✅ Complete
📦 Committed fce9028 (test gate green)
```

Or open the fullscreen command center: `local-ai tui` — header HUD, anime sidekick with 17 states, streaming chat, live plan/tasks, project tree, permission modals with diffs, 6 themes.

## Install

One command — `local-ai` then works from any directory:

```bash
curl -fsSL https://raw.githubusercontent.com/manojkumar20081604-lang/Local-AI/main/install.sh | sh
```

Or locally (needs Rust ≥ 1.77 — https://rustup.rs):

```bash
git clone https://github.com/manojkumar20081604-lang/Local-AI.git
cd Local-AI
./install.sh              # → ~/.local/bin/local-ai (no sudo)
./install.sh --system     # → /usr/local/bin/local-ai (needs sudo)
./install.sh --uninstall  # remove it again
# or: cargo install --path cli
```

If `~/.local/bin` isn't on your PATH (one time only):

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc   # bash
fish_add_path ~/.local/bin                                  # fish
```

You need **one** local provider running: [Ollama](https://ollama.com) (`ollama serve && ollama pull qwen2.5`) or [LM Studio](https://lmstudio.ai) (enable Local Server).

## First run

```text
$ local-ai
No provider configured → pick LM Studio / Ollama / llama.cpp
✓ Connected (health-checked, with retry)
→ pick a model from the live list → saved globally
✓ Provider / ✓ Model / ✓ Project detected
◉ᴗ◉ Ready.
```

Every later launch just prints the three checkmarks. Change anytime with `/model`, `/provider`, `provider select`, or `models select`. Non-interactive setup: `local-ai init --provider ollama --model qwen3-coder` (or `local-ai config set provider ollama`). Diagnose everything with `local-ai doctor` (9 checks, CI-gateable exit code).

## Commands

| Area | Commands |
|---|---|
| Talk | `local-ai` (setup → TUI) · `local-ai "goal"` (agent loop) · `chat` (REPL + `--tools`) · `analyze` |
| Build | `plan` / `exec-plan` · `agent run` · `mission` (persisted, resumable) · `debug` (self-debug loop) |
| Safety | plan/build modes · diff preview + allow-once/session/reject · `git rollback` (stash checkpoints) |
| Know | `index` (hybrid retrieval) · `graph` (symbols/imports) · `memory` (3 tiers) · `LOCAL-AI.md` conventions |
| Ship | `git status/diff/log/blame` · AI `git commit` (test-gated) · `browse` (cited web research) · MCP (`filesystem`, `github`) |
| Improve | `bench` (TPS/TTFT/pass@1) · `dataset collect` · `finetune eval` (deploy gate) · `metrics` · `propose` |
| Setup | `init` · `provider list/select/test` · `models list/select` · `config` · `doctor` · `tui` |

Full reference with examples: [`cli/README.md`](cli/README.md).

## How it stays honest

- **Grounded or silent** — every answer cites real files (`[path:line]`); strict mode exits `2` on hallucination instead of guessing.
- **Never invents edits** — `SEARCH` anchors must match byte-for-byte or the op is refused; paths must exist in inventory; traversal blocked.
- **Never destroys silently** — dangerous commands (`rm -rf /`, `mkfs`, exfil) need `--approve dangerous`; mutating runs snapshot first; verifier reject-rate is a tracked metric (`metrics`).
- **Finetunes can't bypass grounding** — the verifier runs post-deploy on every model output; `finetune eval` blocks regressing adapters.

## Layout

```text
Local-AI/
├── install.sh        # one-command install (local build or curl-pipe remote)
├── cli/              # PRIMARY — Rust binary (clap, tokio, ratatui)
│   ├── src/core/     # provider/ (unified trait + autodetect) · config · setup
│   │                 # intelligence · embeddings/index (hybrid RAG) · symbols
│   │                 # verifier · edits (SEARCH-verified) · git (+checkpoints)
│   │                 # agents (orchestrator) · missions · plan · debug
│   │                 # memory · mcp · bench/dataset/metrics · router · ui_events
│   ├── src/commands/ # chat/analyze/agent/mission/debug · files/exec · git
│   │                 # plan · memory/graph/index · browse · bench/dataset
│   │                 # metrics/propose · init/provider/models/doctor/config · tui
│   ├── src/tui/      # anime engine · themes · ratatui app · agent bridge
│   └── tests/        # 166 tests: agent/debug/git/hallucination/modes/phase3-5
│                     # plan/providers/rag/repl/safety/setup/tui
├── finetune/         # QLoRA pipelines (Unsloth/MLX/Axolotl/torchtune) + verify.py
├── bench/            # benchmark prompt sets + results
├── mcp/              # reference MCP servers (filesystem, github)
├── plan.md           # full phased roadmap (all phases shipped)
├── BENCHMARK.md      # benchmark notes
└── frontend/ + src-tauri/  # legacy GUI (deprecated, reference only)
```

## Develop

```bash
cd cli
cargo test                                        # 166 tests, all offline
cargo clippy --all-targets -- -D warnings        # must stay clean
./target/debug/local-ai doctor                    # end-to-end provider check
```

Config lives in `~/.config/local-ai/config.toml` (global — never inside projects). Project caches (index, traces, missions) live under `~/.cache/local-ai/`.

## Roadmap

Phases 1–5 (agent loop, orchestrator, RAG 2.0, missions/MCP, model lab), bare invocation, edit safety 2.0, anime TUI, and first-run productization are all shipped — see [`plan.md`](plan.md) for the full history and what's next.

## License

MIT — see [LICENSE](LICENSE). Contributions welcome: keep `cargo test` + clippy green.
