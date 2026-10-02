# Local-AI — Roadmap to a Local Autonomous Software Engineer

> **Identity:** Local-AI is a privacy-first, local developer agent that understands your
> codebase, plans tasks, edits files, runs tests, verifies changes, and improves models —
> all with local LLMs (LM Studio / Ollama / llama.cpp).

**Killer demo (the bar for every phase):**

```text
User: "Find why my app crashes when I upload a PDF and fix it."

Local-AI:
🔍 Inspecting project
🧠 Finding relevant files
📋 Creating plan (approval: Approve / Modify / Reject)
✏️ Editing 3 files
🧪 Running tests → ❌ 1 failure
🔧 Reading error → fixing → 🧪 42/42 ✅
🔐 Verifying (no invented files/symbols) → ✅ Complete
📦 Commit: fix(upload): handle PDF mime + size guard
```

If a feature doesn't serve that loop, it doesn't ship.

---

## 0. Where we are (honest inventory)

Already built and documented in `cli/` + `finetune/`:

| Area | Status | Code |
|---|---|---|
| Universal providers (LM Studio SSE, Ollama NDJSON + `/v1` compat, llama.cpp, generic) + autodetect | ✅ done | `cli/src/core/provider/*`, `cli/src/commands/models.rs`, `doctor.rs` |
| Plan / Build modes (`--mode`, env, config, `require_build_mode`) | ✅ done | `cli/src/core/config.rs`, all commands |
| Browser (`web_fetch`) + terminal (`exec`) tools for model | ✅ done | `cli/src/core/tools.rs`, `chat.rs --tools` |
| Project intelligence (intent, relevant-file rank, inventory header) | ✅ done | `cli/src/core/intelligence.rs` |
| Hybrid retrieval (fastembed bge-small / TfIdf / Ollama embed, 0.55+0.35 rank, chunk 1500/200) | ✅ done | `cli/src/core/embeddings.rs`, `index.rs` |
| Safe file read/write (traversal guard), exec (`sh -c`/`cmd /C`) | ✅ done | `cli/src/core/fs.rs`, `commands/files.rs`, `exec.rs` |
| 5-layer anti-hallucination (router, retrieval, constrained prompt, tools, verifier + retry) | ✅ done | `cli/src/core/verifier.rs`, `finetune/verify.py`, `chat.rs`, `analyze.rs` |
| Persistent projects + chat history (`projects.json` per OS) | ✅ done | `cli/src/core/projects.rs` |
| QLoRA pipeline (prepare/train/merge/quantize, Unsloth/MLX/torchtune/Axolotl, GGUF → LM Studio) | ✅ done, just hardened | `cli/src/commands/finetune.rs`, `finetune/*.py` |
| Verifier backends (lexical + groundrails/LettuceDetect/ragground/groundlens) | ✅ done | `finetune/verify.py`, `requirements.txt` |

**Rule going forward: no random chat features. Every addition must plug into
plan → edit → test → verify → commit.**

---

## Phase 0 — Repo hygiene (do FIRST, ~1 day)

The public repo currently ships generated junk. This hurts credibility more than
any missing feature.

**Problems (verified 2026-10-01):**

```text
backend/.venv/            894 tracked files (venv must never be committed)
node_modules/             tracked at repo root
backup/, backups/         old App.tsx copies tracked
frontend/src/*.backup     *.stage0.backup etc. tracked
test-one.txt … test-ten.txt  placeholder junk at root
finetune/__pycache__/     bytecode tracked
```

**Tasks:**

- [ ] `git rm -r --cached backend/.venv node_modules backup backups finetune/__pycache__`
- [ ] `git rm --cached test-*.txt frontend/src/*.backup` (or move real fixtures to `cli/tests/fixtures/`)
- [ ] Harden `.gitignore`:
  ```gitignore
  backend/.venv/
  backup/
  backups/
  test-*.txt
  *.backup
  *.stage*.backup
  __pycache__/
  ```
  (root `node_modules/`, `.venv/` already covered — keep them)
- [ ] Decide `backend/` fate: legacy Tauri Python shim or delete? If dead → remove dir, note in README. If alive → document purpose in `backend/README.md`.
- [ ] Re-tag canonical layout in README:
  ```text
  Local-AI/
  ├── cli/          # PRIMARY — Rust binary
  ├── finetune/     # QLoRA pipelines
  ├── frontend/     # legacy reference (deprecated)
  ├── src-tauri/    # legacy backend (deprecated)
  ├── docs/
  ├── cli/tests/
  ├── README.md / BENCHMARK.md / plan.md
  └── LICENSE
  ```

**Acceptance:** `git ls-files | grep -E "venv|node_modules|backup|\.backup|test-.*txt|__pycache__"` → empty. Fresh `git clone` + `cd cli && cargo build` works with no junk.

---

## Phase 1 — Developer Agent — ✅ COMPLETE 2026-10-01 (1.1 git, 1.4 plan-graph, 1.2 debug, 1.3 commits)

**Goal:** close the loop `understand → plan → edit → test → verify → commit`.
This is the highest-ROI phase. Do this before any multi-agent work.

### 1.1 Git intelligence (`NEW`) — ✅ DONE 2026-10-01

Why first: every later agent (debug, review, mission) needs `diff/log/blame`.

- [x] New module `cli/src/core/git.rs`: `status`, `diff (staged/unstaged/file)`, `log -n`, `blame -L`, `branch`, `stash list`
- [x] New commands:
  ```bash
  local-ai git status --project MyApp
  local-ai git diff --project MyApp [--file src/x.rs] [--staged]
  local-ai git log --project MyApp -n 20
  local-ai git blame --project MyApp --file src/x.rs -L 10,40
  ```
- [x] Respect plan mode: read-only cmds allowed in `plan`; nothing here writes (commit comes in 1.3)
- [x] Inject `git status --short + last 5 commits` into `analyze`/`chat` system prompt (behind `--show-context`)
- [x] Tests: `cli/tests/git.rs` with temp repo fixture (init, commit, modify, assert diff/log parse)

**Acceptance:** `analyze "what broke after last commit?"` cites `git diff` + commit hash.

### 1.2 Self-debugging loop (`NEW`, the HUGE one) — ✅ DONE 2026-10-01

Shipped as `local-ai debug`:

```text
Attempt 1 ❌ (exit 1) → parsed 2 failures → relevant: src/auth.rs → LLM fix → approval → applied
Attempt 2 ✅ tests passed
```

- [x] New command `local-ai debug "fix failing tests" --project MyApp --max-attempts 3 --test-cmd "npm test"`
- [x] Loop in `cli/src/commands/debug.rs` reusing `intelligence::rank_relevant_files`, `fs::run_project_command`, `verifier`
- [x] Failure parsers for Rust (`cargo test`), Node (`npm/vitest/jest`), Python (`pytest`) — `cli/src/core/debug.rs::parse_failures`, offline, unit-tested
- [x] Per-attempt transcript (saved to `~/.cache/local-ai/<id>/debug/*.json` — feeds Phase 5 datasets)
- [x] Plan-mode: `--dry-run`/plan preview shows would-run commands, never executes
- [x] Edit engine `cli/src/core/edits.rs` (SEARCH-verified apply, invented-path refusal, dry-run) — shared with Phase 2 Coder
- [x] Tests: `cli/tests/debug.rs` (11 tests: panic/rustc/pytest/jest parsers + edit apply/refusal/dry-run)

**Acceptance:** loop verified live against Ollama (fail → parse → fix proposal → SEARCH-refusal → transcript); seeded-repo demo pending a real failing fixture.

### 1.3 AI commits + changeset UX — ✅ DONE 2026-10-01

Shipped as `local-ai git commit`:

```text
AI CHANGESET
Files changed: 7 / Lines: +143 -29 / Tests: green (via --test-cmd gate)
Commit: feat(auth): add JWT login
[confirm] → committed a1b2c3d
```

- [x] `local-ai git commit --project MyApp --dry-run` → changeset (staged files, +/-, untracked note) + heuristic `type(scope): subject` message
- [x] `AI CHANGESET` output + approval prompt (`--yes` to skip); conventional-format + outside-changeset-mention guards
- [x] `commit` blocked in plan mode (preview allowed); `--test-cmd` gate aborts on red tests; staged-only semantics
- [x] Tests: conventional validator, message shapes, diff-stat + commit round-trip, plan-mode gate (`cli/tests/git.rs`, 12 tests)

**Acceptance:** end-to-end `edit → debug → verify → commit` without leaving CLI.

### 1.4 Plan Mode 2.0 — execution graph — ✅ DONE 2026-10-01

Shipped as `local-ai plan` + `local-ai exec-plan`:

```text
PLAN — Goal: Add authentication
[1] Inspect project (auto) ↓ [2] Load relevant files (auto) ↓ [3] Modify src/auth.rs (MANUAL)
↓ [4] Run tests `cargo test` (auto, approval-gated) ↓ [5] Verify + security review (MANUAL)
Approve ✓ / Modify ↻ / Reject ✗ / Execute ▶ (exec-plan <path> [--dry-run])
```

- [x] `local-ai plan "add auth" --project MyApp [--json]` → deterministic steps from intent + ranked files, no writes, plan-mode safe
- [x] `--json` emits `{goal, intent, steps[{id, kind, files, cmd, depends_on}]}`; DAG validated (backwards deps, no dup ids)
- [x] `local-ai exec-plan plan.json --dry-run` previews; real run executes inspect/context/test with approval gate (`--yes`), edit/review print as MANUAL; run report saved
- [x] Plans stored in `~/.cache/local-ai/<id>/plans/` (+ `runs/` reports for future missions)
- [x] Tests: `cli/tests/plan.rs` (6 tests: chains, debug reproduce-before-edit, DAG rejection, JSON round-trip)

**Acceptance:** `plan` output is executable by `exec-plan --dry-run` with zero LLM re-planning.

---

## Phase 2 — Agent System (4–8 weeks, after Phase 1) — ✅ COMPLETE 2026-10-02

**Goal:** replace single-LLM calls with orchestrated specialists. Keep it LOCAL — no LangGraph server dependency; a small Rust orchestrator.

```text
User → Planner → {Researcher, Coder, Tester, Debugger, Reviewer} → Verifier → User
```

- [x] `cli/src/core/agents.rs`: `Agent { role, system_prompt, tools_allowed, model_hint }`, `Orchestrator { plan_graph, budget (max steps/tokens), approval_gate }`
- [x] Roles (each maps to existing primitives):
  - Planner → Phase 1.4 graph
  - Researcher → retrieval + `web_fetch` + symbol search (read-only)
  - Coder → `<CREATE_FILE>/<EDIT>` with `SEARCH` verification (build only)
  - Tester/Debugger → Phase 1.2 loop
  - Reviewer → verifier + security checklist (secrets, injection, traversal)
- [x] `local-ai agent run "add auth" --project MyApp --max-steps 20 --approve dangerous`
- [x] Budget + kill-switch: max tool calls, max wall time, never `rm -rf /`, `mkfs`, exfil (`curl POST` with file body) without explicit `--approve dangerous`
- [x] Transcript log `~/.cache/local-ai/<id>/missions/<ts>/trace.jsonl` (every agent I/O) — feeds Phase 4 mission UI + Phase 5 dataset builder
- [x] Tests: golden trace on fixture repo (planner emits 5 steps, coder edit applies, tester passes)

**Acceptance:** `agent run` on fixture repo completes without human input except one approval, trace file exists.
Verified live 2026-10-02 against Ollama qwen2.5:1.5b (plan → research → test → coder fix → re-test → reviewer pass, SEARCH-refusal on bad anchor, trace.jsonl with 12 entries).

**Explicit non-goal:** no autonomous background daemon in this phase. All runs are foreground, bounded, user-invoked.

---

## Phase 3 — Intelligence: memory + code graph + RAG 2.0 (parallelizable after Phase 1) — ✅ COMPLETE 2026-10-02

### 3.1 Three-tier memory (`NEW`) — ✅ DONE 2026-10-02

```text
project-memory.md  User uses React+Rust+Postgres; arch …; important files …
user-memory.md     prefers TypeScript, short explanations, Linux, local models
task-memory/<id>.md  what changed / failed / worked last time
```

- [x] `cli/src/core/memory.rs`: load/merge/save, `local-ai memory show|set|forget --scope project|user|task`
- [x] Auto-update project memory after `git commit` (files changed, stack detected)
- [x] Secrets guard: never persist `*.pem/.env` contents, only paths + redacted hints
- [x] Tests: round-trip set/get, secret redaction test

### 3.2 RAG 2.0 + symbol/dependency graph (`NEW`) — ✅ DONE 2026-10-02

Line-oriented parser (Rust, TS/JS, Python, Go) with the tree-sitter output shape
`{file, lang, defs[{name, kind, line}], imports[]}` — pure Rust, no binary deps.

- [x] `cli/src/core/symbols.rs`: parsers (Rust, TS/JS, Python, +Go) → `{file, lang, defs[{name, kind, line}], imports[], calls[]}` (calls deferred — defs+imports ship)
- [x] Extend `index.json` with `symbols` + `imports` sections; `hybrid_rank` gains `+0.15 symbol_match +0.10 git_recency`
- [x] `local-ai index rebuild` gains `--symbols` (default on, pure-Rust, no binary deps)
- [x] `local-ai graph --project MyApp [--file X]` → `Imports / Imported-by / Functions / Tests / Recent changes`
- [x] Query path: `"where is auth handled?"` resolves `auth() → AuthService → UserController → routes/auth.ts` via graph, not just text
- [x] Tests: fixture monorepo, assert `graph --file` lists known importers

### 3.3 Model router (`NEW`, small) — ✅ DONE 2026-10-02

- [x] `cli/src/core/router.rs`: task → model class: `simple → 3B/7B`, `coding → 9B/14B`, `reason → 20B+`, `embed → embedding model`
- [x] `local-ai chat --route` picks from `models list` by size tag; falls back gracefully; logs choice with `--show-context`
- [x] Config override `router = { coding = "qwen2.5-14b" }`

**Acceptance:** cold-start question on 50k-line repo answers with symbol citations, no full scan.
Verified live 2026-10-02: `graph --query` resolves via symbols, `index status` shows symbols/edges, `chat --route` logs `task class=simple/coding → model` (falls back gracefully), memory redacts `AKIA…` before save. 15 new tests in `cli/tests/phase3.rs`; 110 total green.

---

## Phase 4 — Autonomous Engineering: missions + MCP + browser agent — ✅ COMPLETE 2026-10-02

### 4.1 Mission system (`NEW`, the command-center feel) — ✅ DONE 2026-10-02

```text
MISSION #024 — Build authentication — ████████░░ 80%
✓ Planner ✓ Researcher ✓ Coder ✓ Tester → Security Reviewer
Files: 7 modified | Tests: 31/31 | Current: security review
```

- [x] `local-ai mission create|list|show|resume|cancel` backed by `~/.local/share/.../missions.json` + per-mission `trace.jsonl`
- [x] Mission = persisted Phase 2 orchestrator state (plan graph + step statuses + approvals)
- [x] CLI `--watch` streams step updates; frontend later renders TASKS/CHANGES/TERMINAL/LOGS from same files
- [x] Tests: create → run 2 steps → resume after kill → completes

Step execution extracted into `commands/runner.rs` so `agent run` and
`mission resume` run the identical loop (persisted per step via hook).

### 4.2 MCP tool servers (`NEW`, extensibility) — ✅ DONE 2026-10-02

- [x] Define `Tool` trait already in `tools.rs` → add MCP client (`cli/src/core/mcp.rs`, JSON-RPC over stdio): `list_tools`, `call_tool`
- [x] `local-ai --mcp filesystem --mcp github ...` registers external tools; model sees them alongside built-ins
- [x] Ship two reference servers in `mcp/` (Node or Python): `filesystem`, `github` (read-only first) — Python stdlib, read-only
- [x] Security: MCP writes/exec still gated by plan/build + approval; denylist env exfil

### 4.3 Browser agent (extend `web_fetch`) — ✅ DONE 2026-10-02

- [x] Keep `web_fetch(url)` (10s, truncation) as primitive; add `local-ai browse "stripe checkout api" --max-pages 5` → fetch → extract code blocks → cite URLs
- [x] Visible log: every URL fetched printed with `--show-context`; never silent browsing
- [x] Tests: local HTTP fixture server, assert citation URLs in answer

**Acceptance:** `mission create "add stripe" → agent run` researches docs, implements, tests, all steps visible in `mission show`.
Verified live 2026-10-02: `mission create → resume --max-steps 2 (29%) → resume (done, tests pass)` + `show --watch` exits on terminal; `browse --seed-url` cites `[source: url]` with code blocks; MCP filesystem server `list_tools`/`call_tool`/traversal-refusal live via python3. 9 new tests in `cli/tests/phase4.rs`; 119 total green.

---

## Phase 5 — Model Lab: benchmark + self-improving loop — ✅ COMPLETE 2026-10-02

### 5.1 Benchmark studio (`NEW`) — ✅ DONE 2026-10-02

```text
MODEL LAB — Model | TPS | TTFT | Coding | Reasoning
Qwen 9B   | 34  | 0.8s | 8.2   | 7.5
```

- [x] `local-ai bench --model X --suite coding|reasoning --prompt-set bench/prompts.jsonl` → TPS, TTFT, pass@1 on fixture tasks
- [x] Store results `bench/results/<model>-<ts>.json`, `local-ai bench compare A B`
- [x] Seed `bench/prompts.jsonl` from existing `cli/tests/*` fixtures (no new infra)

### 5.2 Finetune feedback loop (extend existing pipeline) — ✅ DONE 2026-10-02

```text
approved interactions → dataset builder → QLoRA → eval → compare → deploy if better
```

- [x] `local-ai dataset collect --project MyApp --from missions --only-approved` → appends to JSONL (reuse prepare format)
- [x] `local-ai finetune eval --adapter X --base Y --bench bench/prompts.jsonl` → before/after table
- [x] Gate: only `merge→quantize→deploy` if new ≥ old on eval; verifier still runs post-deploy (finetune never bypasses grounding)
- [x] Tests: tiny eval on 5 prompts, assert gate blocks regressing adapter

### 5.3 Self-improvement proposals (the standout feature) — ✅ DONE 2026-10-02

- [x] `local-ai metrics --project MyApp` → retrieval hit-rate, verifier reject-rate, test pass-rate from `trace.jsonl` + `index.json`
- [x] `local-ai propose` → e.g. *"retrieval failed 18% on large TS — suggest AST symbol retrieval (expected +12% hit)"* with `[Inspect] [Apply] [Reject]`
- [x] Proposals are plan-graphs (Phase 1.4), never silent self-modification

**Acceptance:** `finetune eval` blocks a worse adapter; `propose` generates one inspectable improvement from real metrics.
Verified live 2026-10-02: `bench compare` + `finetune eval` gate BLOCKED on 0.80→0.40 regression (exit 1, passes with --allow-regression); `metrics` shows hit/reject/pass rates from traces+index; `propose` emits baseline/index/grounding/debug plans with valid DAGs, `--apply` saves to plans cache only. 15 new tests in `cli/tests/phase5.rs`; 134 total green.

---

## Cross-cutting (applies to every phase)

- [x] **Safety:** plan-mode blocks writes/exec-train/merge/commit; dangerous-command denylist (`rm -rf`, `mkfs`, `curl … | sh`); approval gate for MCP/browser/commit. Document in `cli/README.md`.
  Verified 2026-10-02: `require_build_mode` gates (`config.rs`), `is_dangerous_command` kill-switch (`agents.rs`), MCP screening (`mcp.rs`); all documented in `cli/README.md` (Modes, Browser+Terminal, Agent, MCP sections).
- [x] **Verification:** every agent output through `verifier.rs` + `verify.py`; strict exits 2 (CI-gate like `nogrounds` NG001). Track reject-rate as health metric.
  Verified 2026-10-02: `analyze` strict exits 2 (`analyze.rs:222`), chat retries once then falls back to `Not in context`; runner/debug verify `SEARCH` anchors + invented files; reject-rate tracked via `local-ai metrics`.
- [x] **Testing:** each phase ships `cli/tests/<area>.rs` + fixtures; `cargo test` + `cargo clippy -- -D warnings` green before merge.
  Verified 2026-10-02: 134 tests green (`agent/debug/git/hallucination/modes/phase3/phase4/phase5/plan/providers/rag`); `cargo clippy --all-targets -- -D warnings` green (incl. `main.rs`→lib import structural fix removing duplicate module compilation).
- [ ] **Docs:** update `cli/README.md` per phase; keep root `README.md` as the sales pitch (identity + killer demo + 60s quickstart), move deep docs to `docs/`.
  Partial 2026-10-02: `cli/README.md` current through Phase 5 (bench/dataset/eval/metrics/propose). Remaining: trim root `README.md` to pitch + quickstart, move deep docs to `docs/`.

---

## Sequencing (don't parallelize Phase 1)

```text
NOW:        Phase 0 hygiene (1 day) — unblocks credibility
NEXT (1):   1.1 git → 1.4 plan-graph → 1.2 debug loop → 1.3 commits
THEN (2):   Phase 2 agents (needs 1.x primitives)
THEN (3+4): Phase 3 memory/graph + Phase 4 missions/MCP (can parallelize)
LAST (5):   Phase 5 lab (needs mission traces + dataset volume)
```

**Suggested first PRs (all DONE 2026-10-01):**

1. [x] `chore: untrack venv/node_modules/backups, harden gitignore`
2. [x] `feat: git status/diff/log/blame commands + tests`
3. [x] `feat: plan --json execution graph + exec-plan --dry-run`
4. [x] `feat: debug loop with test parsers + transcript`
5. [x] `feat: changeset + ai commit (plan-gated)`

---

## Open questions (answer before Phase 2)

1. `backend/` — keep or delete? (Assumed dead; confirm.)
2. Frontend (Tauri) — revive later from mission JSON, or CLI-only for Phases 1–2? (Recommend CLI-first, GUI reads same `missions.json`/`trace.jsonl`.)
3. MCP transports — stdio only, or also HTTP/SSE? (Recommend stdio first.)
4. Tree-sitter languages — start with Rust+TS+Python, add Go later? (Recommend 3 first.)
5. Eval set for Phase 5 — reuse `cli/tests` fixtures or new `bench/` set? (Recommend new `bench/prompts.jsonl` seeded from tests.)

---

*Generated 2026-10-01. Source idea dump: multi-agent, plan-graph, self-debug, memory, MCP, git, commits, browser, RAG 2.0, dep-graph, autonomous mode, missions, router, benchmarks, finetune loop, self-improvement. Sequenced against actual repo state (`cli/src/core/*`, `finetune/*`).*
