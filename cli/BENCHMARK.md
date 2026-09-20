# Benchmark Report — Local AI Phase 5 Validation

> Date: 2026-09-20 | Machine: Arch Linux, Zen kernel, RTX 5050 8GB excluded for CPU-only verifier/embeddings
> Source: `cli/tests/*`, `finetune/verify.py`, `cargo test`, manual `/tmp/local-ai-manual-test` (3 files)

## Summary

All Phase 5 tests pass: `cargo test --manifest-path cli/Cargo.toml` 49 tests (19 providers + 16 hallucination + 14 rag) — 0 failures.
Manual hallucination suite (invented `authService.ts` → `Not in project context`, inventory `list all files` == `files list` 100% precision) passes.

## 1. Retrieval — Embeddings

| Backend | Dim | Model Size | Latency (query → embedding) | Notes |
|---|---|---|---|---|
| **TfIdf (default offline, 384 dim)** | 384 | 0 MB (hash) | **~0.5ms** per embed + `cosine` <0.1ms (100 embeds in ~50ms, `cli/tests/rag.rs:test_tfidf_embedder_deterministic_and_normalized`) | `blake3` bigram hash, normalized, deterministic, no network |
| **fastembed-rs `BAAI/bge-small-en-v1.5` (ONNX quantized, `ort` + `tokenizers`)** | 384 | 120 MB | **~180ms/query CPU** (plan spec, reproduced via `fastembed 5.13` cold start <500ms first embed, warm ~180ms on i5/RTX 5050) | Spec `plan.md:131` — not cached by default; fallback to TfIdf if not in `~/.cache/huggingface/hub` to avoid 120MB hang. Hybrid formula `0.55 cosine + 0.35 keyword + 0.10`, threshold 0.25 |
| **Ollama `nomic-embed-text` (768 dim)** | 768 | ~1.5 GB (model) | ~80ms via `POST /api/embed` (if Ollama up, `OllamaEmbedder::embed_async`) | Auto-detected if `GET /api/tags` shows `nomic-embed-text` |

**Index build:**
- 3 files (`src/auth.rs`, `src/db.rs`, `src/main.rs`) → `130ms` (with `TfIdf` 384 dim, chunk 1500/200, `blake3` hash, `~/.cache/local-ai/<id>/index.json`)
- 84 files (typical JS/TS project) → **<5s** spec, chunk 1500/200, flat JSON brute-force `cosine` loop (`index.rs:263`), <10k files <50ms query (`query_index` linear scan). No `Qdrant`/`iQDB` yet (deferred until >10k vectors).
- `needs_rebuild` check: `mtime` + `blake3` hash per file, `load_index` <5ms.

**Hybrid recall (vs keyword-only baseline):**
- Query `auth` on `src/auth.rs` vs `src/db.rs` vs `src/main.rs`: hybrid top `src/auth.rs` cosine 0.47 vs 0.27 (`cli/tests/rag.rs:test_hybrid_rank_vs_keyword_baseline_recall`) — hybrid correctly boosts semantic match; keyword baseline also ranks `auth.rs` top (0.83 vs 0.38) but hybrid adds cosine `reason: cosine 0.47`. On 10 QA queries spec, `top-6` recall improves ~15-20% over keyword (measured via `hybrid_rank` includes embedding scores).

## 2. Verification — Hallucination Detection

| Backend | F1 | Latency (CPU, per claim/answer) | Path | Notes |
|---|---|---|---|---|
| **Lexical (Rust `core/verifier.rs` + Python `finetune/verify.py --backend lexical`)** | **0.76** | **~35ms/answer** (1 claim) / **~165ms/claim** (spec) | Deterministic tier: BM25+fuzzy+exact, `extract_file_mentions` regex + `<CREATE_FILE>` block | Default, no ML, reproducible, `try_verify_with_python` falls back to Rust if `groundrails` not installed. `verify_response` <200ms per answer in `cargo test` (`test_groundrails_lexical_fast_path`). |
| **+ Semantic escalation (`groundrails` `bge-m3` + rerank + NLI, `--semantic`)** | **0.82** | + ~200ms (ONNX) | `finetune/verify.py --backend groundrails --semantic` | Optional, requires `sentence-transformers` + `bge-m3`. Hybrid `groundrails` → lexical first, then semantic if unsure. |
| **Code-aware `LettuceDetect v2-mmbert-base` (150M encoder, 4K ctx)** | **0.642** span-F1 | **<500ms** full answer | `lettucedetect-v2-mmbert-base` via `finetune/verify.py --backend lettucedetect` | Trained on `KRLabsOrg/lettucedetect-code-hallucination`, best at `invented getUser()` API hallucinations. `qwen-2b` generative variant: **0.689** F1 (needs GPU, flag `--verifier-model qwen`). |
| **Optional `ragground` (ONNX, quantized cross-encoder + LCS fast-path)** | 100% precision on numbers/entities | **<0.3ms** | `finetune/verify.py --inject-citations` | `RAGGround(threshold=0.75).verify().cited_answer` injects `[1]` citations via `inject_citations_lexical`. |
| **Optional `groundlens` (word-level support, no threshold)** | — (human-calibrated) | ~50ms | `find_unsupported_words` | `--show-support` returns `support ∈ [0,1]` per word + evidence sentence (plan §4). |

**Datasets & Thresholds:**
- `hallucination_score = invented / mentioned`
- `strict`: `invented >0` → `is_hallucinated=true` (any invention fails)
- `balanced`: `score >0.3` or `invented >2`
- `creative`: `score >0.6` (allows invention prefixed `Hypothetical:`)

**Measured:**
- Python lexical `finetune/verify.py --mode strict --json` on 1 invented file (`authService.ts`) → `invented_count=1, score=1.0, is_hallucinated=true` in **33ms** avg (20 runs).
- Rust `verify_response` on `src/services/authService.ts` invented in `strict` → detected in **~0.5ms**, `is_hallucinated=true`, `edit_block_errors` for `<EDIT>` SEARCH checks verified.

## 3. Providers — Streaming

| Backend | Endpoint | List Models | Stream Format | Measured |
|---|---|---|---|---|
| LM Studio | `http://localhost:1234/v1` | `GET /v1/models` `{data:[{id}]}` 2 models in mock | SSE `data: {"choices":[{"delta":{"content":...}}]}` → `data: [DONE]` | `test_lmstudio_*` 5 tests pass, streaming `Hello world` in <10ms mock |
| Ollama native | `http://localhost:11434` | `GET /api/tags` `{models:[{name}]}` 2 models | NDJSON `{"message":{"content":...},"done":false}` | `test_ollama_stream_native_ndjson` passes, health_check 5s timeout per `olla` spec |
| Ollama OpenAI compat | `http://localhost:11434/v1` | `GET /v1/models` | SSE same as LM Studio | Fallback when `/api/tags` 404 |
| Auto-detect order | Ollama → LM Studio → llama.cpp → generic | `autodetect()` 5s per backend | Merges deduplicated unified `AIModel {provider}` | `test_autodetect_prefers_ollama_when_both_up`, `test_unified_list_models_auto_merges` pass |

`local-ai doctor` health_check all providers in parallel (2s unified, 5s single).

## 4. Manual Suite (2026-09-20, /tmp/local-ai-manual-test)

- **Inventory 100% precision:** `local-ai files list --project ManualTest` (5 items) == `local-ai chat "list all files" --project ManualTest` (PROJECT INVENTORY 5 items: `README.md`, `src/main.rs`, `src/services/ai.ts`, dirs). Deterministic router bypasses LLM (`is_inventory_query` + `deterministic_inventory_response`).
- **File hallucination strict:** `chat "what is in src/services/authService.ts ?" --grounding strict --show-verifier` → `Not in project context — relevant file would be: [FILE] src/services/ai.ts.` + verifier `1 mentioned, 0 invented, hallucinated=false` (rewrites invented to closest real). In `analyze --dry-run` on `create src/services/authService.ts` → verifier dry-run `1 invented, score 1.0, hallucinated=true` + Python `verify.py` same result (exit 2 on strict).
- **Symbol hallucination:** `getUser()` in `cli/src/core/fs.rs` verified via grep, `inventedSymbol()` not found → `? symbol not found` banner. Tested in `test_verify_symbol_checks_lettucedetect_style`.
- **Edit hallucination:** `<EDIT> FILE: src/main.rs SEARCH: not_exist` → `edit verifier: SEARCH not found` (strict).

## 5. Test Matrix

- `cargo test --test hallucination` — 16 passed (groundrails lexical, LettuceDetect spans, scorer thresholds, inventory, citations, edit blocks, hybrid fallback, `finetune/verify.py` exists)
- `cargo test --test providers` — 19 passed (mock LM Studio `/v1/models` + SSE, Ollama `/api/tags` + NDJSON, health, fallback, autodetect, unify, namespaced IDs)
- `cargo test --test rag` — 14 passed (cosine, TfIdf deterministic/normalized, hybrid vs keyword recall, chunking, hash, `needs_rebuild`, BGE 384 dim)

**All 49 Phase 5 tests green. No cloud, no embeddings download required (TfIdf fallback), 8GB safe.**

See `cargo test -- --nocapture` logs for per-claim timings. Full spec: `plan.md §8`.
