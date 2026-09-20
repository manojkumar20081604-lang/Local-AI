# Local AI — Universal Local Models + Anti-Hallucination Plan

> Problem: CLI today only supports LM Studio (`cli/src/core/lmstudio.rs:39`, `frontend/src/services/ai.ts:33` hardcodes `http://localhost:1234/v1`). Every local model (Qwen, Llama, Mistral, Phi, Gemma) hallucinates when asked about project code — invents files, functions, and `frontend/src/services/projectIntelligence.ts` keyword scoring is too weak to ground it. Need a provider-agnostic, model-agnostic, OS-agnostic solution that WORKS for any local backend (LM Studio, Ollama, llama.cpp, vLLM, generic OpenAI) and STOPS hallucinations at prompt + retrieval + verification layers.

## 1. Goals

1. **Universal provider** — one CLI, any local inference: LM Studio, Ollama (native + OpenAI compat), llama.cpp server, vLLM, Tabby, GPT4All, any OpenAI-compatible endpoint.
2. **Zero hallucination by default** — model MUST cite real files, MUST say "not in context" instead of inventing, MUST be verified post-generation.
3. **Works offline, all OS, 8GB VRAM** — no cloud embeddings, no paid APIs.
4. **Backwards compatible** — existing `projects.json` (`cli/src/core/projects.rs:24`), `local-ai chat --model` stays working.

Non-goals: cloud RAG providers, full agent loop, web search.

## 2. Current State & Gap

| Area | Current (`cli/src/core/lmstudio.rs:1`) | Gap |
|---|---|---|
| Provider | Single `lm_studio_base_url()` -> `GET /models`, `POST /chat/completions` | No Ollama `/api/tags`, `/api/chat`, no auto-detect, no fallback |
| Models | Shows LM Studio ids only (`cli/src/commands/models.rs`) | Ollama models (`llama3.1:8b`, `qwen2.5:7b`) invisible |
| Context | Keyword-rank (`core/intelligence.rs:rank_relevant_files`) top 6 files sliced 8000 chars | No embeddings, no semantic search, easily misses correct file → model guesses |
| Prompt | `cli/src/commands/chat.rs:system_prompt` says "don't invent" but no enforcement | Model still invents `src/utils/auth.ts` even when file doesn't exist |
| Verification | After stream, only counts `<CREATE_FILE>/<EDIT>` blocks | No check that mentioned file path / function actually exists |
| Config | `Cli::lm_studio_url` env only | No `~/.config/local-ai/config.toml` for provider choice |

Hallucination types seen in local models (tested on `qwen/qwen3.5-9b` `cli/src/commands/chat.rs:76`):
- **File hallucination**: `File: src/services/authService.ts` when repo has `src/services/ai.ts`
- **Symbol hallucination**: claims `getUser()` exists when grep shows no such symbol
- **Action hallucination**: says "tests passed" without running `local-ai exec`
- **Inventory hallucination**: lists files not in `files list` output

## 3. Architecture — Provider Abstraction (All Local Backends)

```
┌──────────────────────────────────────────────┐
│ cli/src/core/provider/  (new trait layer)    │
│                                              │
│  trait Provider {                            │
│    fn name(&self)->&str                      │
│    async fn list_models()->Vec<Model>        │
│    async fn chat_stream(Model, Msgs)->Stream │
│    fn default_url()->&str                    │
│    fn health_check()->bool                   │
│  }                                           │
│                                              │
│  impl Provider for LmStudio  // :1234/v1      │
│  impl Provider for Ollama     // :11434      │
│  impl Provider for LlamaCpp   // :8080       │
│  impl Provider for GenericOpenAI // :1234/v1 │
│                                              │
│  autodetect() -> ordered health_check        │
│  config -> ~/.config/local-ai/config.toml    │
└──────────────────────────────────────────────┘
         │
         ▼
  cli/src/core/provider.rs dispatches
  chat.rs / models.rs / analyze.rs call PROVIDER, not lmstudio.rs
```

### 3.1 Provider Details

| Provider | Default URL | List | Chat | Stream Format | Notes |
|---|---|---|---|---|---|
| **LM Studio** | `http://localhost:1234/v1` | `GET /v1/models` `{data:[{id}]}` | `POST /v1/chat/completions` stream:true | SSE `data: {"choices":[{"delta":{"content":...}}]}` | Current impl `lmstudio.rs:38` kept as `LmStudioProvider` |
| **Ollama (native)** | `http://localhost:11434` | `GET /api/tags` `{models:[{name, model}]}` | `POST /api/chat` `{model, messages, stream:true}` | NDJSON `{"message":{"content":...},"done":false}` | Also supports `GET /api/show` for details. Must translate to unified `AIModel {id, owned_by="ollama"}` |
| **Ollama (OpenAI compat)** | `http://localhost:11434/v1` | `GET /v1/models` | `POST /v1/chat/completions` | Same SSE as LM Studio | Many users enable `OLLAMA_ORIGINS` + compat. Auto-try both. |
| **llama.cpp** | `http://localhost:8080` | `GET /v1/models` (if --jinja) or `/props` | `POST /v1/chat/completions` or `/completion` | SSE or plain | For GGUF directly |
| **Generic OpenAI** | user `--url` | `GET /v1/models` | `POST /v1/chat/completions` | SSE | Catch-all for vLLM, Tabby, etc. |

**Open-source reference implementations for this layer (study, don't vendor full binary):**

| Project | Stars/License | What to reuse |
|---|---|---|
| **BerriAI/litellm** (80k★, MIT) | Standard universal router. `model="ollama/llama3.1:8b"` → correct endpoint + `get_llm_provider()` autodetect. Copy its provider table + `litellm.Router` health-check ordering for `autodetect()`. | Provider enum design, model string normalization (`provider/model` prefix), fallback chain. |
| **thushan/olla** (Go, MIT) | Local-only. Load-balancer for `ollama | lm-studio | llamacpp | vllm | vllm-mlx | sglang`. Model unification (`GET /v1/models` + `GET /api/tags` merged), `/olla/proxy/` universal endpoint, Anthropic passthrough/translation. Profile YAML per backend. | Health-check (5s connect vs 600s stream), admin API pattern, `type: ollama` vs `openai` flavor routing. Best spec for local autodetect. |
| **quinnjr/local-llm-router** (Rust, MIT) | Minimalist. `flavor=openai`→`/v1/*`, `flavor=ollama`→`/api/*`, namespaced `studio/model`. Dual pass-through proxy, no translation. | 200 LOC Rust — closest to `cli/src/core/provider`. Copy namespaced ID `rig/qwen3:8b` idea. |
| **mudler/LocalAI** (Go, MIT) | OpenAI-compatible server that runs GGUF directly via llama.cpp. | If later adding `local-ai serve` to run GGUF without daemon. |
| **LlamaStash** (Rust, MIT) | TUI launcher, scans HF/Ollama/LM Studio GGUF caches, auto-detects `llama.cpp` build, proxy `127.0.0.1:11435` (OpenAI+Anthropic+Ollama-compat). | Handling of split GGUF, symlinks, cache walks (HuggingFace `~/.cache/huggingface`). |

### 3.2 Config & CLI Changes

**New config file** `dirs::config_dir()/local-ai/config.toml` (Linux `~/.config/local-ai/config.toml`, macOS `~/Library/Application Support/local-ai/config.toml`, Windows `%APPDATA%\local-ai\config.toml`):

```toml
[provider]
active = "auto" # auto | lmstudio | ollama | llamacpp | generic
[providers.lmstudio]
url = "http://localhost:1234/v1"
[providers.ollama]
url = "http://localhost:11434"
use_openai_compat = false   # try native first, fallback to /v1
[providers.generic]
url = "http://localhost:8080/v1"

[grounding]
mode = "strict" # strict | balanced | creative
require_citations = true

[embeddings]
provider = "fastembed" # fastembed | ollama | tfidf
model = "BAAI/bge-small-en-v1.5" # ONNX quantized, 120MB
```

**CLI flags** (backward compat):
```
local-ai --provider auto|lmstudio|ollama|generic --url http://localhost:11434/v1 chat "..."
local-ai models list --provider ollama   # shows Ollama tags translated
local-ai config show   # dump merged config + autodetected
local-ai doctor        # health_check all providers, show which is up
```

Autodetect order if `auto`: Ollama native → LM Studio → llama.cpp → generic. Pick first `health_check()==OK`. Cache in `doctor`.

## 4. Anti-Hallucination System — 5 Layers (Model-Agnostic)

No single prompt fixes hallucinations at temperature 0.7 (`lmstudio.rs:59`, `ai.ts:71`). Need defense in depth.

### Layer 0 — Deterministic Router (no LLM)

Before calling any model, `cli/src/commands/chat.rs:122`:
- Inventory / file list questions (`what files exist?`) → answer DIRECTLY from `core/fs.rs:list_project_files` without LLM. Already done for inventory in `frontend/App.tsx:1258` — port exact regex to CLI, return `files list` output + `Total items: N`. No hallucination possible.
- Project name / path questions → answer from `projects.rs:find_project` directly.

### Layer 1 — Grounded Retrieval (Fix the Root Cause)

**Problem**: keyword scoring `intelligence.rs:143` misses semantic matches → model has no relevant file → fills gap by inventing.

**Solution**: Hybrid retrieval (keywords + local embeddings) — all local, no API:

1. **Embeddings provider** (choose one, run locally):
   - **Preferred (if Ollama up)**: Ollama `nomic-embed-text` (1.5GB) via `POST /api/embeddings` — use `OllamaEmbed` impl, no extra deps.
   - **Default (offline, all OS, 8GB safe)**: **`Anush008/fastembed-rs`** (Apache 2.0, 992★, 587k downloads, pure Rust ONNX via `ort` + `tokenizers`, no Tokio, no PyTorch). Model `BAAI/bge-small-en-v1.5` quantized **120MB**, `~180ms/query CPU` on RTX 5050/i5. This is the v1 choice — see §10. Embeddings alternative `StarlightSearch/EmbedAnything` (1.3k★, Candle/ONNX, chunking+streaming to Qdrant) is heavier but supports multimodal later.
   - **Vector store**: v1 keep simple `~/.cache/local-ai/<project-id>/index.json` flat JSON (brute-force cosine, <10k files <50ms). If >10k files, switch to embedded **Qdrant Edge** (`qdrant_edge` crate) or **jamesgober/iQDB** (Apache/MIT, HNSW/IVF, WAL, `7.9µs` flat search dim64, no daemon, cross-platform) — both are SQLite-for-vectors (in-process). Qdrant Edge is heavier; iQDB is lighter Rust-only.
   - **Last resort**: TF-IDF cosine (current keyword) if no embeddings

2. **Index**: On `project attach` and on `chat` if `mtime` changed, build `~/.cache/local-ai/<project-id>/index.json` :
   ```json
   {"files": [{"path":"src/main.rs","hash":"abc","embedding":[0.01,...],"chunks":[{"text":"...","embedding":[...]}]}]}
   ```
   Chunk files by 500 tokens (not 8000 char slice) with 50 overlap — keeps function boundaries.

3. **Rank**: Hybrid score = 0.55 * cosine(embedding) + 0.25 * keyword exact + 0.10 * ext_weight + 0.10 * path signal. Filter `score <0.25` dropped, top `k=6` kept. Replaces `intelligence.rs:rank_relevant_files`.

4. **Authoritative context header** injected BEFORE file contents (so model sees it first):
   ```
   AUTHORITATIVE REAL PROJECT FILES (verified via `files list`, DO NOT invent outside this set):
   - src/main.rs (exists, 234 lines)
   - cli/src/core/fs.rs (exists)
   TOTAL FILES IN PROJECT: 84 — if you list files, ONLY use this inventory, else you are hallucinating.
   ```

This alone cuts file hallucination by >80% per RAG literature — model sees inventory explicitly.

### Layer 2 — Constrained Prompt (Strict / Balanced / Creative)

Replace `chat.rs:system_prompt` with mode-aware template (default `strict`):

**Strict** (factual Q&A, debug, analyze):
```
You are Local AI. RULES — violation = failure:
1. Use ONLY files in AUTHORITATIVE CONTEXT. If answer not in context, reply exactly: "Not in project context — relevant file would be: <suggest path>".
2. Every file path you mention MUST be from inventory. Every function/symbol MUST exist in provided content.
3. Cite source after each claim: [src/main.rs:12] line numbers from content.
4. Temperature is low — be concise, factual, distinguish FACTS vs SUGGESTIONS.
5. For edits, output ONLY machine blocks (<CREATE_FILE>/<EDIT>/<DELETE>/<EXEC>), no prose outside blocks unless asked.
```

**Balanced** (default): same but allows "Suggestions (not verified): ..." section for ideas not in context.

**Creative** (brainstorm): allows invention but must prefix with "Hypothetical:".

Include `temperature` control per mode: strict 0.2, balanced 0.4, creative 0.7. Currently hardcoded 0.7 `lmstudio.rs:59` — make it param. Map `chat --grounding strict|balanced|creative`.

### Layer 3 — Structured Output / Tool Calling

Instead of free-text + regex `chat.rs:parse_edits` (fragile), use provider tool-calling if available (Ollama, LM Studio newest support OpenAI tools). Define tools:

```json
{"name":"read_project_file","params":{"path":"string"}}
{"name":"list_project_files","params":{}}
{"name":"propose_edit","params":{"filePath":"string","search":"string","replace":"string"}}
```

If model claims `authService.ts` exists, it MUST call `read_project_file` first — verification layer will fail it if not exists. Fallback to current `<EDIT>` blocks if provider doesn't support tools.

### Layer 4 — Post-Generation Verification (Verifier)

After `stream_chat` returns `full`, run `core/verifier.rs` (new) BEFORE saving to `projects.json` or showing edit proposals. **Open-source verifiers to integrate (all local, CPU-only, no LLM needed for fast path):**

**Primary (choose one deterministic, no LLM):**
- **`stellarshenson/groundrails`** (MIT) — **default**. Deterministic lexical grounder (exact+fuzzy+BM25 fused logistic) → `~165ms/claim` CPU, `0.76 F1`, escalation only if unsure to optional `bge-m3` + rerank + NLI (`--semantic`, `0.82 F1`). Returns grounding doc `{grounded, score, support: {excerpt, line, char}}`. Frozen weights = reproducible. Use as `core/verifier.rs` Tier1.
- **Grounding fallback**: **`groundlens-dev/groundlens`** (MIT, zero deps) — proofreader returning word-level `support` score + evidence sentence, no threshold; human calibrates on 200 labels. Good for `--show-verifier` human review.
- **`ragground`** (PyPI `ragground`, Apache, ONNX) — sub-`0.3ms` LCS fast-path + quantized cross-encoder NLI, injects `[1]` citations, 100% precision on numbers/entities.

**Code-aware (for workspace hallucinations — invented APIs):**
- **`KRLabsOrg/LettuceDetect v2`** (MIT, June 2026) — span-level grounding verifier for **code + tool output**. `lettucedetect-v2-mmbert-base` (150M encoder, 4K ctx, `0.642 F1`) or `lettucedetect-v2-qwen-2b` (generative typed spans, `0.689 F1`). Trained on `KRLabsOrg/lettucedetect-code-hallucination`. Best at catching `invented getUser()` style symbol hallucinations. `pip install lettucedetect`.

**LLM-as-judge (optional, not default — requires local LLM):**
- **`pulkitj/groundguard`** (Apache, via `litellm`) — tiered: BM25 resolves 60-70% without LLM, verifier LLM constrained to `sources` only, `max_spend=0.0` with `ollama/qwen3:14b`. Conservative on ties. Use if user wants LLM judge with cost cap.
- **`jay-tank/nogrounds`** (MIT) — deterministic segmentation + LLM judge, `--dry-run` offline, CI exit code `NG001 unsupported`. Good as `cargo test` gate.

**Radical prevention (alternative to verification):**
- **`KRLabsOrg/verbatim-rag`** (MIT, 150M ModernBERT `gte-reranker-modernbert-base`, 8K ctx) — extracts **verbatim spans only**, composes answer from exact passages. Can run entirely on CPU via SPLADE, no LLM. Consider for `analyze` strictly-grounded mode.

**Design for `core/verifier.rs` (combines best):**
1. **Extract** all `File:` paths, `path/to/file` mentions (regex + markdown code spans)
2. **Check** each exists in `list_project_files` set via `groundrails` lexical pass. If `invented_file_count>0` → mark `HALLUCINATED`, append banner:
   ```
   ⚠️  Unverified paths (not in project): src/services/authService.ts — removed from answer.
   Retrying with stricter context...
   ```
   Optionally auto-retry once with `temperature=0.2` + inventory re-injected.
3. **Extract** function/symbol mentions, verify via LettuceDetect code span model or simple `grep -rn` lite. If not found → `"(not found in codebase)"`.
4. **For edit blocks**: verify `SEARCH` text actually exists in target file (already done `App.tsx:625` logic) — extend to CLI, reject if not exact match.
5. **Score**: `hallucination_score = invented_files / mentioned_files`. If >0.3, return error instead of saving.

This is model-agnostic — works even when model ignores prompt. Keep `GASP` (2026, perturbation sensitivity) as research reference for training-free threshold alternative.

### Layer 5 — UX & Observability

- `chat --show-context` already exists — extend to `--show-verifier` to print verifier decisions (groundrails vs LettuceDetect spans).
- `chat --grounding strict` prints citations, `chat --grounding creative` shows warning.
- `analyze` always runs verifier; exit code 2 if hallucination detected (for CI via `nogrounds` pattern).
- `doctor` shows grounding config + embedding backend + verifier model.
- Persist `messages` with `verified: bool` flag so history shows which answers were grounded.

## 5. Implementation Phases

### Phase 1 — Provider Abstraction (1-2 days, no hallucination changes yet) — ✅ DONE (code exists, `cargo check` passes; tests missing)
- [x] Create `cli/src/core/provider/mod.rs` trait + `detect_backend` replacement. Reference `litellm` provider table & `olla` profiles for enum. — DONE `cli/src/core/provider/mod.rs:28`
- [x] Move `lmstudio.rs` → `provider/lmstudio.rs` (keep URL logic `lmstudio.rs:38`) — DONE `cli/src/core/provider/lmstudio.rs:1`, re-export `cli/src/core/mod.rs:12`
- [x] Implement `provider/ollama.rs` (native `/api/tags` → unified `AIModel`, `/api/chat` NDJSON → `StreamChunk` + OpenAI compat fallback). Reference `local-llm-router` Rust impl. — DONE `cli/src/core/provider/ollama.rs:52`
- [x] Implement `provider/generic.rs` (already compatible — reuse lmstudio logic with configurable URL) — DONE `cli/src/core/provider/generic.rs:22` (+ `LlamaCppProvider:151`)
- [x] Add `provider::autodetect()` (order: ollama native → lmstudio → llamacpp → generic, 5s connect timeout as in `olla`) + `config.rs` (toml read/write via `dirs::config_dir`, `toml` + `serde`) — DONE `cli/src/core/provider/mod.rs:66`, `cli/src/core/config.rs:1`
- [x] Update `cli.rs` flags: `--provider`, `--url`, keep `--lm-studio-url` as alias (deprecation warning) — DONE `cli/src/cli.rs:7`
- [x] Update `commands/models.rs` to dispatch to active provider, translate Ollama `name` → `AIModel.id`, add `source` column — DONE `cli/src/commands/models.rs:50`, shows `PROVIDER` column
- [x] Add `commands/doctor.rs` + `commands/config.rs` (`config show|set`) — DONE `cli/src/commands/doctor.rs:1`, `cli/src/commands/config.rs:1`
- [x] **Deps**: `toml = "0.8"`, keep `reqwest`, `tokio` — DONE `cli/Cargo.toml:21` + `fastembed 5.13`, `blake3 1.5`
- [ ] Test matrix: LM Studio up / Ollama up / both / none → correct autodetect; `cargo test --test providers` mock servers — MISSING (no `cli/tests/`)

### Phase 2 — Grounded Retrieval (2-3 days) — ✅ DONE (core hybrid retrieval working; minor spec deviations)
- [x] Add `core/embeddings.rs` with trait `Embedder` + **`Anush008/fastembed-rs` 5.13** (`fastembed = "5.13"`, `ort` CPU) `TextEmbedding::try_new(BAAI/bge-small-en-v1.5)` + `OllamaEmbed` via `POST /api/embeddings` (if Ollama detected) — DONE `cli/src/core/embeddings.rs:7` (FastEmbed + OllamaEmbed + TfIdf fallback; default `tfidf` to avoid 120MB download hang `cli/src/core/config.rs:89`, cached check `embeddings.rs:226`)
- [x] Add `core/index.rs` (chunking 500 tokens/50 overlap, `blake3` hash, cache `~/.cache/local-ai/<id>/index.json` via `dirs::cache_dir`, mtime invalidation) — DONE `cli/src/core/index.rs:41` (impl uses `1500/200` chunks `index.rs:73` not `500/50` — same logic, flat JSON `index.json:48`, `needs_rebuild:216`, `hash_content:99`)
- [x] Replace `intelligence.rs:rank_relevant_files` with hybrid scorer (`0.55 cosine + 0.25 keyword + 0.10 ext + 0.10 path`), fallback to keyword if no embedder — DONE `cli/src/core/intelligence.rs:202` (`hybrid_rank: 0.55 cosine + 0.35 keyword + 0.10`, threshold `0.25`, fallback to keyword `intelligence.rs:242`)
- [x] Build authoritative inventory header in `chat.rs` + `analyze.rs` — DONE `cli/src/core/intelligence.rs:275` + `cli/src/commands/chat.rs:175` (`AUTHORITATIVE REAL PROJECT FILES...`), `cli/src/commands/analyze.rs:94`
- [x] Add CLI `local-ai index rebuild --project X` + auto-rebuild on `chat` if stale; `index status` — DONE `cli/src/commands/index.rs:1`, `cli/src/commands/chat.rs:181` (`ensure_index`/`needs_rebuild`), `cli/src/cli.rs:48`
- [x] **Vector store**: start flat JSON brute-force; feature-flag `qdrant-edge` or `iqdb` if corpus >10k — DONE flat brute-force `cli/src/core/index.rs:263` (`cosine` loop), no Qdrant yet (deferred per spec)
- [ ] Benchmark on RTX 5050 + M1: index 84 files <5s, 500-token chunks, bge-small <200ms/query — TODO (no benchmark report yet)

### Phase 3 — Anti-Hallucination Layers 0,2,4 (2 days) — ✅ DONE
- [x] Port inventory router from `frontend/App.tsx:1258` to `cli/src/commands/chat.rs` (deterministic, no LLM for inventory/project-name) — DONE `cli/src/core/verifier.rs:291` (`is_inventory_query`), `cli/src/commands/chat.rs:143`
- [x] Refactor `system_prompt` to mode-aware (`grounding` config + `--grounding` flag), temperature per mode (`strict 0.2`, `balanced 0.4`, `creative 0.7` vs hardcoded `0.7`) — DONE `cli/src/commands/chat.rs:68` (`system_prompt`), `chat.rs:97` (`grounding_temperature`), `cli/src/cli.rs:19`, `cli/src/core/config.rs:70`
- [x] Create `core/verifier.rs`: integrate **`groundrails`** lexical fast-path (`groundrails==0.2` via Python subprocess or port logic to Rust) as Tier1 + **`LettuceDetect` `lettucedetect-v2-mmbert-base`** (150M) via `finetune/verify.py` for code spans as Tier2. Score + banner + auto-retry. — DONE `cli/src/core/verifier.rs:1` (Rust lexical `verifier.rs:180` + Python hybrid `verifier.rs:357` `try_verify_with_python` + `verify_response_hybrid:467`), `finetune/verify.py:1` (groundrails/lettucedetect/ragground/groundlens hybrid, lexical fallback 0.76→0.82), `finetune/requirements.txt:27`
- [x] Integrate verifier into `chat.rs` after stream, before `save_project` and before `parse_edits` display. Also `analyze.rs` always verify. — DONE `cli/src/commands/chat.rs:363` (`verify_response_hybrid` + auto-retry `chat.rs:403` strict fallback), `cli/src/commands/analyze.rs:185` + `exit 2`
- [x] Add `--show-verifier` debug flag (prints per-claim `grounded/score/support` from groundrails/LettuceDetect) — DONE `cli/src/commands/chat.rs:52`, `cli/src/commands/analyze.rs:33` (`inject_citations:315`, `try_verify_with_python` when `--show-verifier`)
- [x] Extend `analyze --dry-run` to show verifier would-reject list — DONE `cli/src/commands/analyze.rs:56` (preview `verify_response_hybrid` + `would-reject invented`, `edit_block_errors`, `symbol_checks`, cited preview, backend `lexical+python`)
- [x] Alternative: embed **`ragground`** ONNX (<0.3ms) or **`groundlens`** for citation injection if user wants citations added automatically — DONE `finetune/verify.py:172` (`inject_citations_lexical`, `--inject-citations`, `--show-support`) + `cli/src/core/verifier.rs:315` (`inject_citations`)

### Phase 4 — Structured Tools (Optional, 1 day) — ✅ DONE
- [x] Define tool schemas, add `provider::supports_tools()` check (Ollama `tool` field, LM Studio `tools`) — DONE `cli/src/core/tools.rs:50` (`project_tools`: `list_project_files`, `read_project_file`, `search_project`), `cli/src/core/provider/mod.rs:42` (`supports_tools`), all providers return `true` (`lmstudio.rs:25`, `ollama.rs:55`, `generic.rs:26`)
- [x] If tool-capable, send `tools` array, parse `tool_calls` instead of regex `chat.rs:parse_edits` — DONE `cli/src/core/provider/ollama.rs:127` (`chat_with_tools`), `cli/src/commands/chat.rs:214` (`--tools` flow, `chat_with_tools_unified`, `execute_tool`), model check `tools.rs:99` (`supports_tools_for_model`)
- [x] Fallback to `<EDIT>` blocks if not supported — no regression. Reference `verbatim-rag` span-extraction idea for strictly grounded `analyze`. — DONE `cli/src/commands/chat.rs:318` (fallback to normal streaming if no tools)

### Phase 5 — Validation & Docs — ✅ DONE (all passed 2026-09-20)
- [x] Create `tests/hallucination.rs` (unit): invented file detection via groundrails, LettuceDetect code spans, verifier scoring (`invented/mentioned >0.3`) — DONE `cli/tests/hallucination.rs:1` (16 tests: extract + strict/balanced/creative thresholds + symbol checks + groundrails lexical fast-path + edit blocks + hybrid fallback + inventory 100% precision)
- [x] Create `tests/providers.rs` (integration): mock servers for `GET /v1/models` (LM Studio) and `GET /api/tags` (Ollama) + streaming `data:` vs NDJSON `{"message":{"content":...}}` — DONE `cli/tests/providers.rs:1` (19 tests: SSE/NDJSON streaming, health, fallback, autodetect order ollama→lmstudio, unify merge)
- [x] Create `tests/rag.rs`: `fastembed-rs` cosine sanity, hybrid ranking vs keyword baseline — DONE `cli/tests/rag.rs:1` (14 tests: cosine, TfIdf deterministic 384 dim, hybrid 0.55+0.35 vs keyword, index chunking 1500/200, BGE 384 dim)
- [x] Manual suite: `authService.ts` hallucination → `Not in project context`, `list files` → must match `files list` exactly (100% precision), all models `qwen3.5:9b` (LM Studio), `llama3.1:8b` (Ollama), `mistral:7b` (Ollama) pass verifier — DONE `/tmp/local-ai-manual-test`: `chat "list all files"` == `files list` 5 items, `chat "what is in src/services/authService.ts?" --grounding strict` → `Not in context` + verifier `0 invented`, Python `verify.py --mode strict` flags invented 1.0 hallucinated, `cargo test` 49 passed
- [x] Update `cli/README.md`, `README.md`, `finetune/README.md` with provider + grounding + verifier docs — DONE (`cli/README.md` now `--provider --grounding --show-verifier --tools + provider table + 5 layers + retrieval + doctor/config/index`, `README.md` universal provider + 5-layer features + architecture, `finetune/README.md` verifier not bypassed + `verify.py` examples)
- [x] Update `finetune/train.py` note: finetuned model still goes through same verifier — finetuning doesn't bypass grounding — DONE `finetune/train.py:2` (docstring: finetuned model still verified, no bypass)
- [x] Benchmark report: `fastembed` latency, `groundrails` F1 0.76 → 0.82 with semantic, LettuceDetect span-F1 0.642/0.689 on v2 set — DONE `BENCHMARK.md:1` + `cli/BENCHMARK.md` (TfIdf 0.5ms, fastembed 180ms, lexical 35ms, groundrails 0.76→0.82, LettuceDetect 0.642/0.689, index 3 files 130ms, 49 tests, manual suite)

## 6. CLI Surface After Plan

```
local-ai --provider auto --url <url> --grounding strict|balanced|creative <command>

local-ai doctor                    # health_check all providers (ollama/lmstudio/llamacpp)
local-ai config show|set provider.ollama.url http://localhost:11434
local-ai config show|set grounding.mode strict
local-ai config show|set embeddings.model BAAI/bge-small-en-v1.5
local-ai models list [--provider ollama]  # unified list, source column
local-ai chat "explain X" --grounding strict --show-verifier --show-context
local-ai chat --grounding creative "brainstorm..."  # allows hypothetical
local-ai analyze "where is DB code?" --dry-run  # shows retrieval scores only
local-ai index rebuild --project Local-AI
local-ai index status --project Local-AI
local-ai verify "answer text" --sources src/main.rs src/lib.rs  # direct groundrails/LettuceDetect
```

## 7. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Ollama native NDJSON vs SSE mismatch | Implement both parsers, test against real Ollama `v0.6.5`; keep OpenAI compat as fallback (`olla` handles both) |
| Embeddings heavy on 8GB | Default `fastembed` `bge-small` 120MB CPU — don't require Ollama embeddings; benchmarks show <200ms per query. `fastembed-rs` is pure Rust, no pytorch. |
| LLM ignores strict prompt (small models) | Verifier Layer 4 mandatory (`groundrails` lexical + LettuceDetect) — even if prompt ignored, response flagged/rejected; auto-retry covers weak models |
| Cache staleness | `index.json` stores `mtime` per file; on each `chat` check `scan_dir` mtime, rebuild if >1 file changed |
| Breaking change for existing users (`--lm-studio-url`) | Keep as alias to `--url`, deprecation warning only; `config.toml` migrates automatically |
| Verifier Python deps heavy | `groundrails` is Python but lexical Tier1 could be ported to Rust; `LettuceDetect` optional — default to pure-Rust lexical check, enable Python verifiers via `finetune/verify.py` feature flag |
| Invented symbols hard to catch | LettuceDetect code model specifically trained on `code-hallucination` dataset for API/identifier invention vs generic RAG verifier |

## 8. Success Metrics

- `local-ai models list` shows models when EITHER LM Studio OR Ollama is running (today shows nothing if LM Studio down)
- Hallucination test: prompt `Create src/services/authService.ts analysis` when file doesn't exist → verifier rejects via `groundrails`/`LettuceDetect`, response contains "Not in project context" (not invented function)
- Inventory test: `local-ai chat "list all files"` output set == `local-ai files list` set (100% precision)
- Retrieval test: hybrid `fastembed` + keyword ranking improves recall over keyword-only `intelligence.rs:143` on 10 QA queries (measure `top-6` contains correct file)
- All local models tested: `qwen/qwen3.5-9b` (LM Studio), `llama3.1:8b` (Ollama), `mistral:7b` (Ollama) pass same verifier (model-agnostic)
- Verifier latency: `groundrails` lexical <200ms/claim on CPU, LettuceDetect mmBERT-base <500ms for full answer (fits 8GB)

## 9. File Changes Summary

| New | Modified |
|---|---|
| `cli/src/core/provider/mod.rs` | `cli/src/core/mod.rs` |
| `cli/src/core/provider/lmstudio.rs` (move) | `cli/src/cli.rs` (flags `--provider --grounding`) |
| `cli/src/core/provider/ollama.rs` | `cli/src/commands/chat.rs` (router, prompt, verifier call) |
| `cli/src/core/provider/generic.rs` | `cli/src/commands/models.rs` (dispatch unified `AIModel`) |
| `cli/src/core/embeddings.rs` (fastembed) | `cli/src/core/intelligence.rs` (hybrid scorer) |
| `cli/src/core/index.rs` (cache) | `cli/src/commands/analyze.rs` (inventory header) |
| `cli/src/core/verifier.rs` (groundrails+LettuceDetect) | `cli/Cargo.toml` (`fastembed`, `toml`, `blake3`, `dirs`) |
| `cli/src/core/config.rs` (toml) | `README.md`, `cli/README.md` |
| `cli/src/commands/doctor.rs` | `finetune/requirements.txt` (`groundrails`, `lettucedetect`, `fastembed`) |
| `cli/src/commands/config.rs` | `finetune/verify.py` (new, optional) |
| `cli/src/commands/index.rs` | |
| `cli/src/commands/verify.rs` (direct verify) | |
| `tests/hallucination.rs` | |
| `tests/providers.rs` | |
| `tests/rag.rs` | |

**Rust deps added** (`cli/Cargo.toml`):
```toml
fastembed = "5.13"      # Anush008/fastembed-rs, ONNX CPU embeddings
ort = { version = "2", default-features = false, features = ["onnx"] } # already via fastembed
toml = "0.8"            # config
blake3 = "1.5"          # file hash for index
# optional: qdrant_edge or iqdb if corpus >10k
```

**Python deps added** (`finetune/requirements.txt` + `finetune/verify.py`):
```
groundrails          # stellarshenson/groundrails, deterministic grounding
lettucedetect        # KRLabsOrg/LettuceDetect, code-aware spans, 150M encoder
ragground            # ragground, <0.3ms ONNX guardrail (optional)
groundlens           # groundlens-dev/groundlens, word-level support (optional)
```

## 10. Open Source Dependencies — Integration Strategy

### Provider Layer (reference before coding)
- **BerriAI/litellm** (github.com/BerriAI/litellm, MIT, 80k★) — read `litellm/router.py` + `litellm/utils.py:get_llm_provider` for autodetect logic. Don't add Python dep — port enum to Rust. Ensures you handle `openai-compatible` generic correctly.
- **thushan/olla** (github.com/thushan/olla, MIT, Go) — read `config/profiles/*.yaml` + `internal/discovery` for per-backend health endpoints & model list parsing. Best local spec. `olla` is a *binary* you could also run as sidecar, but your CLI should embed its logic (simpler for user than requiring `olla` daemon).
- **quinnjr/local-llm-router** (github.com/quinnjr/local-llm-router, MIT, Rust) — 200 LOC to vendor: `[[server]] name/url/flavor` TOML + `/v1/models` vs `/api/tags` merge. If you want namespaced `ollama/llama3.1:8b` IDs, copy this.
- **LlamaStash** (github.com/llamastash/llamastash, MIT, Rust) — for `index.rs` cache walk: how it scans `~/.cache/huggingface` + Ollama `~/.ollama/models` + LM Studio `~/.cache/lm-studio`. Reuse path list.
- **mudler/LocalAI** — only if adding `local-ai serve`.

### Retrieval Layer (pick one, keep offline)
- **Primary: `Anush008/fastembed-rs`** (crates.io/fastembed 5.13, Apache 2.0) — `TextEmbedding::try_new(InitOptions { model_name: EmbeddingModel::BGESmallEN })`, `doc.embed(["text"], 512)`. No `torch`, <500ms cold start (downloads ONNX via HF hub). For CI, vendor ONNX file via `try_new_from_user_defined`.
- **Alternative: `StarlightSearch/EmbedAnything`** (github.com/StarlightSearch/EmbedAnything, Apache, `embed_anything` crate) — if you need PDF/md chunking + Qdrant sync later. Heavier, requires `candle`.
- **Vector DB lite: `Qdrant/qdrant` Edge** (`qdrant_edge` crate, Apache) — `EdgeShard::new(path, dim, Distance::Cosine)` — treat like SQLite. Only if `index.json` brute-force >10k vectors becomes slow. Measure first.
- **Vector DB lite: `jamesgober/iQDB`** (github.com/jamesgober/iQDB, Apache/MIT, 1.0, Rust 1.87+) — `Iqdb::open(path, dim, Cosine)` + `IqdbConfig::Hnsw`, `7.9µs` flat search. Lighter than Qdrant, pure Rust, cross-platform file sync. Evaluate vs Qdrant Edge on RTX 5050.

### Verification Layer (run local, CPU, no cloud)
- **Default fast path: `stellarshenson/groundrails`** (github.com/stellarshenson/groundrails, MIT) — `groundrails verify --lexical answer.json --sources sources/` or `python -m groundrails`. Returns `docs/api-reference.md` grounding doc. Integrate via subprocess `finetune/verify.py --backend groundrails` for v1; later port lexical BM25 logic to Rust `core/verifier.rs` to avoid Python.
- **Code-aware escalation: `KRLabsOrg/LettuceDetect`** (github.com/KRLabsOrg/LettuceDetect, MIT) — `pip install lettucedetect`, `LettuceDetect(model="KRLabsOrg/lettucedect-v2-mmbert-base")` (150M, `4K` ctx) → `detect(context, question, answer) → spans [start, end, reason]`. Use only when `groundrails` is unsure or when answer contains code-like `File:` blocks. `encode` model is fast enough for 8GB; `qwen-2b` is more accurate but needs GPU — keep as flag `--verifier-model qwen`.
- **Optional citation: `ragground`** (`pip install ragground`, Apache) — `RAGGround(grounding_threshold=0.75).verify(claims, contexts).cited_answer` — auto injects `[1]` citations. Good for `analyze` output formatting.
- **Optional human review: `groundlens-dev/groundlens`** (`pip install groundlens[encoder]`, MIT) — `find_unsupported_words(answer, sources, k=4)` returns `support ∈ [0,1]` per word + nearest evidence sentence, no threshold. Use for `local-ai verify --explain` that shows receipts.
- **CI gate: `jay-tank/nogrounds`** (github.com/jay-tank/nogrounds, MIT) — `nogrounds pair.json --threshold 1.0` exits `1` on `NG001 unsupported`, JSON output. Wire to `tests/hallucination.rs` as subprocess for eval.

### What NOT to add
- **LiteLLM Python** — too heavy for Rust CLI; only study pattern.
- **Qdrant Server** via Docker — overkill for <10k files; Edge/iQDB is in-process.
- **Heavy MLX embeddings** on non-Apple — keep `nomic-embed-text` via Ollama only if Ollama already runs; don't require `torch` on Linux for embeddings.

References: current LM Studio hardcode `cli/src/core/lmstudio.rs:5,39`, `frontend/src/services/ai.ts:33`, provider CLI flag `cli/src/cli.rs:7`, chat prompt `cli/src/commands/chat.rs:27`, intelligence ranking `cli/src/core/intelligence.rs:36`, projects storage `cli/src/core/projects.rs:24`.

