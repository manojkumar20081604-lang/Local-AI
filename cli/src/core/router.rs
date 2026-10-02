//! Model router — task → model class (Phase 3.3, small).
//!
//! Picks the cheapest capable local model without a network round-trip:
//!
//! ```text
//! simple → ≤8B    (run/test/list, fast)
//! coding → 7–15B  (edit/debug, needs code accuracy)
//! reason → ≥14B   (architecture/planning, largest available)
//! embed  → embedding model (retrieval)
//! ```
//!
//! Size tags are parsed from the model id (`qwen2.5:1.5b`, `llama3.1:8b`,
//! `qwen3:32b`, `Qwen/Qwen2.5-7B-Instruct`). Unknown sizes sort as 7B
//! (middle) so routing still works. `local-ai chat --route` logs the choice
//! with `--show-context`; config `[router]` overrides any class.

use serde::{Deserialize, Serialize};

/// Model capability class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelClass {
    Simple,
    Coding,
    Reason,
    Embed,
}

impl std::fmt::Display for ModelClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Simple => "simple",
            Self::Coding => "coding",
            Self::Reason => "reason",
            Self::Embed => "embed",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for ModelClass {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "simple" => Ok(Self::Simple),
            "coding" | "code" => Ok(Self::Coding),
            "reason" | "reasoning" => Ok(Self::Reason),
            "embed" | "embedding" => Ok(Self::Embed),
            _ => Err(format!("unknown class '{}', expected simple|coding|reason|embed", s)),
        }
    }
}

/// Classify a user task into a model class (keyword rules, offline).
/// Mirrors `intelligence::detect_intent` but maps to compute needs.
pub fn classify_task(text: &str) -> ModelClass {
    let t = text.to_lowercase();
    // Embed first: explicit retrieval/embedding requests.
    if t.contains("embed") {
        return ModelClass::Embed;
    }
    // Reason: architecture, planning, review — needs the biggest model.
    for k in ["architecture", "plan", "design", "review", "audit", "refactor", "explain", "why does", "how does"] {
        if t.contains(k) {
            return ModelClass::Reason;
        }
    }
    // Coding: edit/debug/implement — needs code accuracy.
    for k in ["edit", "fix", "bug", "error", "crash", "implement", "add ", "create", "refactor", "debug", "failing", "compile"] {
        if t.contains(k) {
            return ModelClass::Coding;
        }
    }
    // Run/test/list/find are cheap.
    ModelClass::Simple
}

/// Parse `~B` size tag from a model id. Returns billions of params.
/// `None` when no tag is present (caller treats as 7.0 middle).
///
/// Handles `qwen2.5:1.5b`, `llama3.1:8b`, `qwen3:32b`, `Qwen-7B`, `14b`, `70B`.
pub fn parse_size_billions(model_id: &str) -> Option<f32> {
    let lower = model_id.to_lowercase();
    // Scan for `<number>b` tokens (also `m` for small, converted to B).
    let bytes = lower.as_bytes();
    let mut i = 0;
    let mut found: Option<f32> = None;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() || (bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let num_str = &lower[start..i];
            // Skip version fragments like `2.5` in `qwen2.5:` — they are
            // followed by `:` or `-` + letters, not by a size suffix.
            let suffix = lower[i..].chars().next();
            match suffix {
                Some('b') => {
                    if let Ok(n) = num_str.parse::<f32>() {
                        // Guard: `v1`, `v2` handled since no 'b' follows; `32b` ok.
                        // Reject absurd sizes (>1000B) which are version noise.
                        if n > 0.0 && n <= 1000.0 {
                            found = Some(n);
                        }
                    }
                    i += 1;
                }
                Some('m') => {
                    if let Ok(n) = num_str.parse::<f32>() {
                        if (100.0..=100000.0).contains(&n) {
                            found = Some(n / 1000.0);
                        }
                    }
                    i += 1;
                }
                _ => {}
            }
        } else {
            i += 1;
        }
    }
    found
}

fn size_of(model_id: &str) -> f32 {
    parse_size_billions(model_id).unwrap_or(7.0)
}

/// Pick the best model id for a class from `available` (ordered by preference).
/// `overridden` (config `[router]`) wins when it matches an available model
/// (case-insensitive substring) or is returned as-is for explicit ids.
pub fn pick_model(
    class: ModelClass,
    available: &[String],
    overridden: Option<&str>,
) -> Option<String> {
    if available.is_empty() {
        // No inventory — honour an explicit override, else nothing to route.
        return overridden.map(|s| s.to_string());
    }
    if let Some(want) = overridden {
        if !want.trim().is_empty() {
            // Substring match against inventory (e.g. `qwen2.5-14b` matches full id).
            let wl = want.to_lowercase();
            if let Some(hit) = available.iter().find(|m| m.to_lowercase().contains(&wl)) {
                return Some(hit.clone());
            }
            return Some(want.to_string());
        }
    }
    match class {
        ModelClass::Embed => {
            // Prefer names with `embed`; else smallest (fast fallback).
            if let Some(hit) = available.iter().find(|m| m.to_lowercase().contains("embed")) {
                return Some(hit.clone());
            }
            available.iter().min_by(|a, b| size_of(a).partial_cmp(&size_of(b)).unwrap()).cloned()
        }
        ModelClass::Simple => {
            // Cheapest ≤8B; else smallest available.
            let mut small: Vec<&String> = available.iter().filter(|m| size_of(m) <= 8.0).collect();
            small.sort_by(|a, b| size_of(a).partial_cmp(&size_of(b)).unwrap());
            small.into_iter().next().cloned().or_else(|| {
                available.iter().min_by(|a, b| size_of(a).partial_cmp(&size_of(b)).unwrap()).cloned()
            })
        }
        ModelClass::Coding => {
            // 7–15B sweet spot; else closest to 9B.
            let in_band: Vec<&String> = available
                .iter()
                .filter(|m| {
                    let s = size_of(m);
                    (7.0..=15.0).contains(&s)
                })
                .collect();
            if !in_band.is_empty() {
                return in_band.into_iter().min_by(|a, b| (size_of(a) - 9.0).abs().partial_cmp(&(size_of(b) - 9.0).abs()).unwrap()).cloned();
            }
            available.iter().min_by(|a, b| (size_of(a) - 9.0).abs().partial_cmp(&(size_of(b) - 9.0).abs()).unwrap()).cloned()
        }
        ModelClass::Reason => {
            // Largest available (≥14B preferred, else max).
            let mut big: Vec<&String> = available.iter().filter(|m| size_of(m) >= 14.0).collect();
            if !big.is_empty() {
                big.sort_by(|a, b| size_of(b).partial_cmp(&size_of(a)).unwrap());
                return big.into_iter().next().cloned();
            }
            available.iter().max_by(|a, b| size_of(a).partial_cmp(&size_of(b)).unwrap()).cloned()
        }
    }
}
