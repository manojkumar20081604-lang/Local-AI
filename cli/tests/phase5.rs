//! Phase 5 tests — benchmark studio (5.1) + dataset loop + eval gate (5.2)
//! + metrics/propose (5.3).
//!
//! All offline and deterministic (no LLM, no network, no provider).

use local_ai::core::fs::ProjectFile;

// ---------------------------------------------------------------------------
// 5.1 bench: prompts, scoring, TPS/TTFT, summarize/compare/gate
// ---------------------------------------------------------------------------

fn bench_prompts_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("bench")
        .join("prompts.jsonl")
}

#[test]
fn test_bench_load_seeded_prompts() {
    use local_ai::core::bench::load_prompts;
    let path = bench_prompts_path();
    let prompts = load_prompts(&path).expect("bench/prompts.jsonl loads");
    // Seeded from cli/tests fixtures: 5 prompts, coding+reasoning suites.
    assert_eq!(prompts.len(), 5, "{:?}", prompts.iter().map(|p| &p.id).collect::<Vec<_>>());
    let suites: std::collections::HashSet<&str> = prompts.iter().map(|p| p.suite.as_str()).collect();
    assert!(suites.contains("coding"), "{:?}", suites);
    assert!(suites.contains("reasoning"), "{:?}", suites);
    // Comments + blanks are skipped (no new infra).
    assert!(prompts.iter().all(|p| !p.id.is_empty() && !p.kind.is_empty()));
}

#[test]
fn test_bench_score_kinds() {
    use local_ai::core::bench::{score_prompt, BenchPrompt};
    let mk = |kind: &str, expect: Vec<&str>| BenchPrompt {
        id: "t".into(),
        suite: "coding".into(),
        kind: kind.into(),
        user: "u".into(),
        expect: expect.into_iter().map(|s| s.to_string()).collect(),
    };
    // contains_any (case-insensitive).
    let p = mk("contains_any", vec!["auth"]);
    assert!(score_prompt(&p, "see src/AUTH.rs", None).0);
    assert!(!score_prompt(&p, "nothing here", None).0);
    // contains_all.
    let p = mk("contains_all", vec!["<EDIT>", "FILE:"]);
    assert!(score_prompt(&p, "<EDIT> FILE: x", None).0);
    assert!(!score_prompt(&p, "<EDIT> only", None).0);
    // verifier_clean: needs caller-computed verifier_ok (fails closed).
    let p = mk("verifier_clean", vec![]);
    assert!(score_prompt(&p, "anything", Some(true)).0);
    assert!(!score_prompt(&p, "anything", Some(false)).0);
    let (pass, note) = score_prompt(&p, "anything", None);
    assert!(!pass);
    assert!(note.contains("--project"), "note: {}", note);
    // Unknown kinds fail closed.
    let p = mk("mystery", vec![]);
    let (pass, note) = score_prompt(&p, "anything", None);
    assert!(!pass);
    assert!(note.contains("unknown"), "note: {}", note);
}

#[test]
fn test_bench_tps_and_measure_sync() {
    use local_ai::core::bench::{estimate_tps, measure_sync};
    // ~4 chars/token convention.
    assert!((estimate_tps(400, 1.0) - 100.0).abs() < 0.01);
    assert_eq!(estimate_tps(100, 0.0), 0.0);
    // measure_sync: TTFT to first chunk, TPS over whole call.
    let (out, ttft_ms, tps, secs) = measure_sync(|cb| {
        cb("hello ");
        cb("world");
        "hello world".to_string()
    });
    assert_eq!(out, "hello world");
    assert!(secs >= 0.0);
    assert!(tps >= 0.0);
    let _ = ttft_ms;
}

#[test]
fn test_bench_summarize_compare_gate() {
    use local_ai::core::bench::{gate_verdict, BenchResult, PromptResult};
    let pr = |id: &str, pass: bool, tps: f32| PromptResult {
        id: id.into(),
        suite: "coding".into(),
        secs: 1.0,
        ttft_ms: 100,
        tps,
        chars: 100,
        pass,
        note: String::new(),
    };
    let base = BenchResult::summarize("base-model", "coding", vec![pr("a", true, 10.0), pr("b", true, 20.0)]);
    assert!((base.pass_at_1 - 1.0).abs() < 0.001);
    assert!((base.mean_tps - 15.0).abs() < 0.01);
    // Adapter regressed on one prompt → gate BLOCKS deploy.
    let worse = BenchResult::summarize("adapter", "coding", vec![pr("a", true, 12.0), pr("b", false, 5.0)]);
    let (ok, line) = gate_verdict(&base, &worse);
    assert!(!ok, "line: {}", line);
    assert!(line.contains("BLOCKED"), "line: {}", line);
    // Ties pass (≥ is enough to deploy).
    let tied = BenchResult::summarize("adapter2", "coding", vec![pr("a", true, 9.0), pr("b", true, 9.0)]);
    let (ok, line) = gate_verdict(&base, &tied);
    assert!(ok, "line: {}", line);
    assert!(line.contains("PASS"), "line: {}", line);
    // Compare rows align by prompt id.
    let rows = local_ai::core::bench::compare(&base, &worse);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "a");
    assert!(rows[0].a_pass && rows[0].b_pass);
    assert!(!rows[1].b_pass);
}

#[test]
fn test_bench_save_load_round_trip() {
    use local_ai::core::bench::{load_result, save_result, BenchResult, PromptResult};
    let dir = std::env::temp_dir().join(format!("local-ai-bench-{}", uuid::Uuid::new_v4()));
    let result = BenchResult::summarize("qwen-test", "all", vec![PromptResult {
        id: "x".into(),
        suite: "coding".into(),
        secs: 0.5,
        ttft_ms: 50,
        tps: 20.0,
        chars: 40,
        pass: true,
        note: "ok".into(),
    }]);
    let path = save_result(&dir, &result).expect("save");
    assert!(path.exists());
    let back = load_result(&path).expect("load");
    assert_eq!(back.model, "qwen-test");
    assert_eq!(back.prompts.len(), 1);
    assert!((back.pass_at_1 - 1.0).abs() < 0.001);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5.2 dataset: chat pairing, mission harvest, render, append
// ---------------------------------------------------------------------------

fn chat_msg(role: &str, content: &str) -> local_ai::core::projects::ChatMessage {
    local_ai::core::projects::ChatMessage { role: role.into(), content: content.into() }
}

#[test]
fn test_dataset_chat_pairing_and_approval() {
    use local_ai::core::dataset::collect_chat_samples;
    let msgs = vec![
        chat_msg("user", "fix login"),
        chat_msg("assistant", "here is the fix"),
        chat_msg("user", "thanks"),
        // Orphaned user turn (no assistant reply) is skipped.
        chat_msg("system-note", "ignored"),
    ];
    let always = |_: &str| true;
    let samples = collect_chat_samples(&msgs, "sys", false, &always);
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].user, "fix login");
    assert_eq!(samples[0].source, "chat");
    // --only-approved keeps verifier-clean turns only.
    let clean = |s: &str| !s.contains("HALLUCINATED");
    let msgs2 = vec![
        chat_msg("user", "q1"),
        chat_msg("assistant", "good answer"),
        chat_msg("user", "q2"),
        chat_msg("assistant", "HALLUCINATED src/nope.ts"),
    ];
    let kept = collect_chat_samples(&msgs2, "sys", true, &clean);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].user, "q1");
}

#[test]
fn test_dataset_mission_harvest_approval() {
    use local_ai::core::dataset::collect_mission_samples;
    use local_ai::core::agents::TraceEntry;
    let propose = |output: &str| TraceEntry {
        ts: "2026-01-01T00:00:00Z".into(),
        mission_id: "m1".into(),
        agent: "coder".into(),
        action: "propose".into(),
        input: "fix login".into(),
        output: output.into(),
        status: "proposed".into(),
    };
    let apply_done = TraceEntry {
        ts: "2026-01-01T00:00:00Z".into(),
        mission_id: "m1".into(),
        agent: "coder".into(),
        action: "apply".into(),
        input: "s1".into(),
        output: "{}".into(),
        status: "done".into(),
    };
    // Approved = propose with <EDIT> + later apply done.
    let entries = vec![propose("<EDIT>FILE: a SEARCH: x REPLACE: y </EDIT>"), apply_done];
    let kept = collect_mission_samples(&entries, "sys", true);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].source, "missions");
    // Without the apply, --only-approved drops it; without the flag it stays.
    let entries2 = vec![propose("<EDIT>FILE: a SEARCH: x REPLACE: y </EDIT>")];
    assert!(collect_mission_samples(&entries2, "sys", true).is_empty());
    assert_eq!(collect_mission_samples(&entries2, "sys", false).len(), 1);
    // Non-edit proposes (chat text) are never samples.
    let entries3 = vec![propose("i think the bug is over there")];
    assert!(collect_mission_samples(&entries3, "sys", false).is_empty());
}

#[test]
fn test_dataset_render_and_append() {
    use local_ai::core::dataset::{append_jsonl, render_sample, DatasetSample};
    let s = DatasetSample { system: "sys".into(), user: "u".into(), assistant: "a".into(), source: "chat".into() };
    let v = render_sample(&s, "sharegpt");
    assert!(v.get("messages").is_some());
    assert!(v.get("text").and_then(|t| t.as_str()).map(|t| t.contains("sys")).unwrap_or(false));
    let v = render_sample(&s, "alpaca");
    assert!(v.get("instruction").is_some());
    assert!(v.get("output").is_some());
    // Append creates + appends JSONL (reuse `finetune prepare` format).
    let path = std::env::temp_dir().join(format!("local-ai-dataset-{}.jsonl", uuid::Uuid::new_v4()));
    let n = append_jsonl(&path, &[s.clone(), s], "sharegpt").unwrap();
    assert_eq!(n, 2);
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content.lines().count(), 2);
    assert!(append_jsonl(&path, &[], "sharegpt").unwrap() == 0);
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// 5.2 eval gate: tiny 5-prompt eval blocks a regressing adapter
// ---------------------------------------------------------------------------

#[test]
fn test_eval_gate_blocks_regressing_adapter_on_five_prompts() {
    use local_ai::core::bench::{gate_verdict, BenchResult, PromptResult};
    // Simulate a 5-prompt bench (mirrors bench/prompts.jsonl size).
    let mk = |model: &str, passes: &[bool]| {
        BenchResult::summarize(model, "all", passes.iter().enumerate().map(|(i, p)| PromptResult {
            id: format!("p{}", i),
            suite: "coding".into(),
            secs: 1.0,
            ttft_ms: 100,
            tps: 20.0,
            chars: 80,
            pass: *p,
            note: String::new(),
        }).collect())
    };
    let base = mk("base", &[true, true, true, true, false]); // 0.80
    let worse = mk("adapter-bad", &[true, true, false, false, false]); // 0.40
    let (ok, line) = gate_verdict(&base, &worse);
    assert!(!ok, "gate must block: {}", line);
    let better = mk("adapter-good", &[true, true, true, true, true]); // 1.00
    let (ok, _) = gate_verdict(&base, &better);
    assert!(ok);
}

// ---------------------------------------------------------------------------
// 5.3 metrics: pure stats + proposals are plan-graphs
// ---------------------------------------------------------------------------

fn test_project(tag: &str) -> local_ai::core::projects::Project {
    local_ai::core::projects::Project {
        id: format!("test-phase5-{}", tag),
        name: "phase5-fixture".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        folder_path: None,
        messages: Vec::new(),
    }
}

fn trace(agent: &str, action: &str, status: &str) -> local_ai::core::agents::TraceEntry {
    local_ai::core::agents::TraceEntry {
        ts: "2026-01-01T00:00:00Z".into(),
        mission_id: "m1".into(),
        agent: agent.into(),
        action: action.into(),
        input: "in".into(),
        output: "out".into(),
        status: status.into(),
    }
}

#[test]
fn test_metrics_trace_stats_rates() {
    use local_ai::core::metrics::trace_stats;
    let entries = vec![
        trace("researcher", "context", "done"),
        trace("researcher", "context", "partial"),
        trace("reviewer", "review", "pass"),
        trace("reviewer", "review", "flagged"),
        trace("coder", "propose", "proposed"),
        trace("coder", "apply", "refused"),
        trace("tester", "test", "pass"),
        trace("tester", "test", "fail"),
    ];
    let s = trace_stats(&entries);
    assert_eq!(s.context_total, 2);
    assert_eq!(s.context_done, 1);
    assert_eq!(s.review_total, 2);
    assert_eq!(s.review_flagged, 1);
    assert_eq!(s.coder_propose_total, 1);
    assert_eq!(s.coder_refused, 1);
    assert_eq!(s.tests_total, 2);
    assert_eq!(s.tests_pass, 1);
    // Manual/blocked tester states are workflow, not test outcomes.
    let s2 = trace_stats(&[trace("tester", "test", "manual"), trace("tester", "test", "blocked")]);
    assert_eq!(s2.tests_total, 0);
}

#[test]
fn test_metrics_empty_proposes_baseline() {
    use local_ai::core::metrics::{build_proposals, merge_metrics};
    let proj = test_project("empty");
    let m = merge_metrics(&proj, &[], &[], 0, &[], None);
    assert!(!m.has_data());
    assert!(m.retrieval_hit_rate.is_none());
    assert!(m.verifier_reject_rate.is_none());
    assert!(m.test_pass_rate.is_none());
    let files = vec![ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false }];
    let props = build_proposals(&m, &proj, &files);
    assert_eq!(props.len(), 1);
    assert_eq!(props[0].id, "collect-baseline");
    // Every proposal carries a valid plan-graph (DAG, no dup ids).
    local_ai::core::plan::validate_graph(&props[0].plan).unwrap();
}

#[test]
fn test_metrics_bad_retrieval_proposes_index() {
    use local_ai::core::metrics::{build_proposals, merge_metrics};
    let proj = test_project("retrieval");
    // 18% retrieval failure (≈ plan text) → symbol-retrieval proposal.
    let mut entries = Vec::new();
    for i in 0..11 {
        entries.push(trace("researcher", "context", if i < 9 { "done" } else { "partial" }));
    }
    entries.push(trace("tester", "test", "pass"));
    let m = merge_metrics(&proj, &[], &entries, 1, &[], Some((10, 50, 5, Some(true))));
    let hit = m.retrieval_hit_rate.unwrap();
    assert!(hit < 0.85, "hit {}", hit);
    let files = vec![ProjectFile { name: "a.ts".into(), path: "src/a.ts".into(), is_directory: false }];
    let props = build_proposals(&m, &proj, &files);
    assert!(props.iter().any(|p| p.id == "rebuild-index-symbols"), "{:?}", props.iter().map(|p| &p.id).collect::<Vec<_>>());
    let p = props.iter().find(|p| p.id == "rebuild-index-symbols").unwrap();
    assert!(p.expected_gain.contains("+12%"), "{}", p.expected_gain);
    local_ai::core::plan::validate_graph(&p.plan).unwrap();
}

#[test]
fn test_metrics_high_reject_proposes_grounding() {
    use local_ai::core::metrics::{build_proposals, merge_metrics};
    let proj = test_project("grounding");
    let entries = vec![
        trace("researcher", "context", "done"),
        trace("reviewer", "review", "flagged"),
        trace("reviewer", "review", "flagged"),
        trace("coder", "propose", "proposed"),
        trace("tester", "test", "pass"),
    ];
    let m = merge_metrics(&proj, &[], &entries, 1, &[], Some((5, 10, 2, Some(false))));
    assert!(m.verifier_reject_rate.unwrap() > 0.10);
    let files = vec![ProjectFile { name: "a.rs".into(), path: "src/a.rs".into(), is_directory: false }];
    let props = build_proposals(&m, &proj, &files);
    assert!(props.iter().any(|p| p.id == "tighten-grounding"), "{:?}", props.iter().map(|p| &p.id).collect::<Vec<_>>());
}

#[test]
fn test_metrics_failing_tests_propose_debug() {
    use local_ai::core::metrics::{build_proposals, merge_metrics};
    let proj = test_project("testing");
    let entries = vec![
        trace("researcher", "context", "done"),
        trace("reviewer", "review", "pass"),
        trace("tester", "test", "pass"),
        trace("tester", "test", "fail"),
    ];
    let m = merge_metrics(&proj, &[], &entries, 1, &[], Some((5, 10, 2, Some(false))));
    assert!(m.test_pass_rate.unwrap() < 1.0);
    let files = vec![ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false }];
    let props = build_proposals(&m, &proj, &files);
    assert!(props.iter().any(|p| p.id == "shrink-edits-debug"), "{:?}", props.iter().map(|p| &p.id).collect::<Vec<_>>());
    for p in &props {
        local_ai::core::plan::validate_graph(&p.plan).unwrap();
        assert!(!p.apply_hint.is_empty());
    }
}

#[test]
fn test_metrics_all_green_proposes_growth() {
    use local_ai::core::metrics::{build_proposals, merge_metrics};
    let proj = test_project("green");
    let entries = vec![
        trace("researcher", "context", "done"),
        trace("researcher", "context", "done"),
        trace("reviewer", "review", "pass"),
        trace("coder", "propose", "proposed"),
        trace("tester", "test", "pass"),
    ];
    let m = merge_metrics(&proj, &[], &entries, 1, &[], Some((5, 10, 2, Some(false))));
    assert_eq!(m.test_pass_rate, Some(1.0));
    let files = vec![ProjectFile { name: "main.rs".into(), path: "src/main.rs".into(), is_directory: false }];
    let props = build_proposals(&m, &proj, &files);
    assert!(props.iter().any(|p| p.id == "expand-bench"), "{:?}", props.iter().map(|p| &p.id).collect::<Vec<_>>());
}
