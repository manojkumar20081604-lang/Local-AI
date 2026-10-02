//! Dataset feedback loop — approved interactions → QLoRA JSONL (Phase 5.2).
//!
//! ```text
//! approved interactions → dataset builder → QLoRA → eval → compare → deploy if better
//! ```
//!
//! `dataset collect` harvests two approved-interaction sources into the same
//! ShareGPT/​Alpaca JSONL that `finetune prepare` emits (so `finetune train`
//! consumes it unchanged):
//!
//! - `chat`: consecutive user→assistant turns from `projects.json` history.
//! - `missions`: coder `propose` outputs from mission `trace.jsonl` files.
//!
//! `--only-approved` keeps only verifier-clean samples (chat turns that pass
//! `verifier.rs`; mission proposes followed by a successful `apply`).
//! Finetuning never bypasses grounding: the verifier still runs post-deploy
//! on every model output.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use super::agents::TraceEntry;

/// One harvested sample before rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetSample {
    pub system: String,
    pub user: String,
    pub assistant: String,
    pub source: String,
}

impl DatasetSample {
    fn valid(&self) -> bool {
        !self.user.trim().is_empty() && !self.assistant.trim().is_empty()
    }

    fn capped(mut self, max_chars: usize) -> Self {
        self.user.truncate_capped(max_chars);
        self.assistant.truncate_capped(max_chars);
        self
    }
}

trait TruncateCapped {
    fn truncate_capped(&mut self, n: usize);
}

impl TruncateCapped for String {
    fn truncate_capped(&mut self, n: usize) {
        if self.len() > n {
            let mut end = n;
            while end > 0 && !self.is_char_boundary(end) {
                end -= 1;
            }
            self.truncate(end);
            self.push_str("…[truncated]");
        }
    }
}

/// Pair chat history into user→assistant samples.
/// Non-paired messages (system notes, orphaned turns) are skipped.
pub fn collect_chat_samples(
    messages: &[super::projects::ChatMessage],
    system: &str,
    only_approved: bool,
    is_approved: &dyn Fn(&str) -> bool,
) -> Vec<DatasetSample> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        if messages[i].role == "user" {
            if let Some(reply) = messages.get(i + 1) {
                if reply.role == "assistant" {
                    if !only_approved || is_approved(&reply.content) {
                        let sample = DatasetSample {
                            system: system.to_string(),
                            user: messages[i].content.clone(),
                            assistant: reply.content.clone(),
                            source: "chat".to_string(),
                        };
                        if sample.valid() {
                            out.push(sample.capped(8000));
                        }
                    }
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Harvest coder proposes from one mission trace.
/// A propose counts as approved when a later `coder/apply` entry in the same
/// trace has status `done` (i.e. the edit actually landed).
pub fn collect_mission_samples(entries: &[TraceEntry], system: &str, only_approved: bool) -> Vec<DatasetSample> {
    // Statuses of all coder applies in this trace (any step).
    let apply_done = entries.iter().any(|e| e.agent == "coder" && e.action == "apply" && e.status == "done");
    let mut out = Vec::new();
    for e in entries {
        if e.agent != "coder" || e.action != "propose" {
            continue;
        }
        if !e.output.contains("<EDIT>") && !e.output.contains("<CREATE_FILE>") {
            continue;
        }
        if only_approved && !(e.status == "proposed" && apply_done) {
            continue;
        }
        let sample = DatasetSample {
            system: system.to_string(),
            user: e.input.clone(),
            assistant: e.output.clone(),
            source: "missions".to_string(),
        };
        if sample.valid() {
            out.push(sample.capped(8000));
        }
    }
    out
}

/// Parse a `trace.jsonl` file, skipping malformed lines.
pub fn read_trace_entries(path: &Path) -> Vec<TraceEntry> {
    let content = fs::read_to_string(path).unwrap_or_default();
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<TraceEntry>(line).ok())
        .collect()
}

/// Render one sample in the `finetune prepare` formats (ShareGPT default).
pub fn render_sample(sample: &DatasetSample, format: &str) -> serde_json::Value {
    let text = format!("system: {}\nuser: {}\nassistant: {}", sample.system, sample.user, sample.assistant);
    match format {
        "alpaca" => serde_json::json!({
            "instruction": sample.user,
            "input": "",
            "output": sample.assistant,
            "text": text,
        }),
        _ => serde_json::json!({
            "messages": [
                {"role": "system", "content": sample.system},
                {"role": "user", "content": sample.user},
                {"role": "assistant", "content": sample.assistant},
            ],
            "text": text,
        }),
    }
}

/// Append samples as JSONL (creates the file when missing). Returns count.
pub fn append_jsonl(path: &Path, samples: &[DatasetSample], format: &str) -> Result<usize> {
    use std::io::Write;
    if samples.is_empty() {
        return Ok(0);
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    for s in samples {
        writeln!(f, "{}", serde_json::to_string(&render_sample(s, format))?)?;
    }
    Ok(samples.len())
}
