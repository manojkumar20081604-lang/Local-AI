//! Benchmark studio — TPS, TTFT, pass@1 on fixture tasks (Phase 5.1).
//!
//! ```text
//! MODEL LAB — Model | TPS | TTFT | Coding | Reasoning
//! Qwen 9B   | 34  | 0.8s | 8.2   | 7.5
//! ```
//!
//! `bench/prompts.jsonl` (seeded from `cli/tests/*` fixtures, no new infra)
//! drives `bench run`, which streams each prompt against a local model and
//! records tokens/sec, time-to-first-token and pass@1. Results land in
//! `bench/results/<model>-<ts>.json`; `bench compare A B` prints the
//! before/after table that `finetune eval` gates on.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// One benchmark prompt (one JSONL line in `bench/prompts.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchPrompt {
    pub id: String,
    pub suite: String,
    pub kind: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub expect: Vec<String>,
}

/// Per-prompt measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResult {
    pub id: String,
    pub suite: String,
    pub secs: f64,
    pub ttft_ms: u64,
    pub tps: f32,
    pub chars: usize,
    pub pass: bool,
    pub note: String,
}

/// Whole-run result file (`bench/results/<model>-<ts>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub model: String,
    pub suite: String,
    pub created_at: String,
    pub prompts: Vec<PromptResult>,
    pub pass_at_1: f32,
    pub mean_tps: f32,
    pub mean_ttft_ms: u64,
}

impl BenchResult {
    pub fn summarize(model: &str, suite: &str, mut prompts: Vec<PromptResult>) -> Self {
        prompts.sort_by(|a, b| a.id.cmp(&b.id));
        let n = prompts.len().max(1) as f32;
        let pass_at_1 = prompts.iter().filter(|p| p.pass).count() as f32 / n;
        let mean_tps = prompts.iter().map(|p| p.tps).sum::<f32>() / n;
        let mean_ttft_ms = prompts.iter().map(|p| p.ttft_ms).sum::<u64>() / prompts.len().max(1) as u64;
        Self {
            model: model.to_string(),
            suite: suite.to_string(),
            created_at: Utc::now().to_rfc3339(),
            prompts,
            pass_at_1,
            mean_tps,
            mean_ttft_ms,
        }
    }
}

/// Load prompt set (one JSON object per line; `#` comments + blanks skipped).
pub fn load_prompts(path: &Path) -> Result<Vec<BenchPrompt>> {
    let content = fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let mut out = Vec::new();
    for (n, line) in content.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        out.push(serde_json::from_str::<BenchPrompt>(t).with_context(|| format!("{}:{} invalid prompt", path.display(), n + 1))?);
    }
    if out.is_empty() {
        anyhow::bail!("No prompts in {}", path.display());
    }
    Ok(out)
}

/// Score one model output. Pure and offline.
///
/// - `contains_any`: output contains any of `expect` (case-insensitive).
/// - `contains_all`: output contains all of `expect`.
/// - `verifier_clean`: `verifier_ok` (computed by the caller via
///   `verifier.rs`, which needs the project inventory).
///
/// Unknown kinds fail closed (`pass=false`, note explains why).
pub fn score_prompt(prompt: &BenchPrompt, output: &str, verifier_ok: Option<bool>) -> (bool, String) {
    match prompt.kind.as_str() {
        "contains_any" => {
            let lower = output.to_lowercase();
            match prompt.expect.iter().find(|e| lower.contains(&e.to_lowercase())) {
                Some(hit) => (true, format!("contains {:?}", hit)),
                None => (false, format!("missing any of {:?}", prompt.expect)),
            }
        }
        "contains_all" => {
            let lower = output.to_lowercase();
            match prompt.expect.iter().find(|e| !lower.contains(&e.to_lowercase())) {
                None => (true, "contains all expected markers".to_string()),
                Some(miss) => (false, format!("missing {:?}", miss)),
            }
        }
        "verifier_clean" => match verifier_ok {
            Some(true) => (true, "verifier: no invented files".to_string()),
            Some(false) => (false, "verifier: hallucinated".to_string()),
            None => (false, "verifier not run (needs --project)".to_string()),
        },
        other => (false, format!("unknown prompt kind {:?}", other)),
    }
}

/// Tokens/sec estimate from streamed text (∼4 chars/token, OpenAI convention).
pub fn estimate_tps(chars: usize, secs: f64) -> f32 {
    if secs <= 0.0 {
        return 0.0;
    }
    (chars as f64 / 4.0 / secs) as f32
}

/// Measure a streaming call: TTFT to first chunk, TPS over the whole call.
/// Sync helper for offline tests; the live `bench run` path measures inline
/// (async sink) to satisfy the provider's `Send` bound without HRTB issues.
pub fn measure_sync(run: impl FnOnce(&mut dyn FnMut(&str)) -> String) -> (String, u64, f32, f64) {
    let start = Instant::now();
    let mut first_ms: Option<u64> = None;
    let mut chars = 0usize;
    let mut on_chunk = |chunk: &str| {
        if first_ms.is_none() {
            first_ms = Some(start.elapsed().as_millis() as u64);
        }
        chars += chunk.len();
    };
    let output = run(&mut on_chunk);
    let secs = start.elapsed().as_secs_f64();
    let ttft_ms = first_ms.unwrap_or((secs * 1000.0) as u64);
    let out_len = output.len();
    (output, ttft_ms, estimate_tps(chars.max(out_len), secs), secs)
}

/// Async variant kept for backwards compat (offline tests only).
/// Takes pre-collected chunks instead of a streaming closure to avoid
/// HRTB `&mut dyn FnMut + Send` lifetime invariance.
pub async fn measure<F, Fut>(mut run: F) -> (String, u64, f32, f64)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = (String, Vec<String>)>,
{
    let start = Instant::now();
    let (output, chunks) = run().await;
    // Simulate TTFT as time to first chunk (approx: start→now if any chunks).
    let secs = start.elapsed().as_secs_f64();
    let ttft_ms = if chunks.is_empty() {
        (secs * 1000.0) as u64
    } else {
        // Best-effort: first chunk arrived early; report elapsed/2 as TTFT
        // when the caller doesn't track it (offline path only).
        ((secs * 1000.0) / (chunks.len().max(1) as f64)) as u64
    };
    let chars: usize = chunks.iter().map(|c| c.len()).sum::<usize>().max(output.len());
    (output, ttft_ms, estimate_tps(chars, secs), secs)
}

/// Default results dir: `<cwd>/bench/results` (override with `--out`).
pub fn default_results_dir() -> Result<PathBuf> {
    let dir = std::env::current_dir()?.join("bench").join("results");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn safe_model_slug(model: &str) -> String {
    let slug: String = model
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.split('-').filter(|w| !w.is_empty()).take(4).collect::<Vec<_>>().join("-");
    if slug.is_empty() { "model".to_string() } else { slug }
}

/// Persist a result; returns the file path.
pub fn save_result(dir: &Path, result: &BenchResult) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let ts = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(format!("{}-{}.json", safe_model_slug(&result.model), ts));
    fs::write(&path, serde_json::to_string_pretty(result)?)?;
    Ok(path)
}

pub fn load_result(path: &Path) -> Result<BenchResult> {
    let content = fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("{} is not a bench result", path.display()))
}

/// Gate verdict: adapter must be ≥ base on pass@1 (ties pass).
/// Returns `(pass, summary_line)`.
pub fn gate_verdict(base: &BenchResult, adapter: &BenchResult) -> (bool, String) {
    let ok = adapter.pass_at_1 >= base.pass_at_1;
    let line = format!(
        "base {} pass@1 {:.2} → adapter {} pass@1 {:.2} — {}",
        base.model,
        base.pass_at_1,
        adapter.model,
        adapter.pass_at_1,
        if ok { "GATE PASS (deploy allowed)" } else { "GATE BLOCKED (adapter regressed)" }
    );
    (ok, line)
}

/// One comparison row for `bench compare`.
#[derive(Debug, Clone)]
pub struct CompareRow {
    pub id: String,
    pub a_pass: bool,
    pub b_pass: bool,
    pub a_tps: f32,
    pub b_tps: f32,
}

pub fn compare(a: &BenchResult, b: &BenchResult) -> Vec<CompareRow> {
    let mut rows = Vec::new();
    for pa in &a.prompts {
        if let Some(pb) = b.prompts.iter().find(|p| p.id == pa.id) {
            rows.push(CompareRow {
                id: pa.id.clone(),
                a_pass: pa.pass,
                b_pass: pb.pass,
                a_tps: pa.tps,
                b_tps: pb.tps,
            });
        }
    }
    rows.sort_by(|x, y| x.id.cmp(&y.id));
    rows
}
