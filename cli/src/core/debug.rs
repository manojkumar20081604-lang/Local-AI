//! Failure parsers for the self-debugging loop (Phase 1.2).
//!
//! Pure functions: test output in, structured failures out. Covers
//! Rust (`cargo test` / rustc), Node (jest/vitest/mocha stacks),
//! Python (`pytest` tracebacks) and a generic `file:line` fallback.
//! Everything is offline and fully unit-tested.

/// One extracted failure location.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Failure {
    /// Repo-relative or absolute path as printed by the tool.
    pub file: Option<String>,
    pub line: Option<u32>,
    /// Human message (panic text, assertion, error header…).
    pub message: String,
    /// `panic` | `compile` | `traceback` | `stack` | `test` | `location`.
    pub kind: String,
}

impl Failure {
    fn new(kind: &str, file: Option<String>, line: Option<u32>, message: String) -> Self {
        let mut m = message.trim().to_string();
        if m.len() > 300 {
            m.truncate(300);
            m.push('…');
        }
        Self { file, line, message: m, kind: kind.to_string() }
    }
}

const CODE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "rb", "php",
    "cs", "cpp", "cc", "c", "h", "hpp", "swift", "toml", "json", "yaml", "yml",
];

fn looks_like_code_path(p: &str) -> bool {
    if p.len() > 220 || p.contains(' ') || p.starts_with("http") {
        return false;
    }
    // Strip line/col suffix first when checking the extension.
    let base = p.split(':').next().unwrap_or(p);
    match base.rsplit('.').next() {
        Some(ext) => CODE_EXTS.contains(&ext.to_lowercase().as_str()),
        None => false,
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok()
}

/// Split `path:line:col` / `path:line` tail. Returns (path, line).
fn split_file_line(token: &str) -> Option<(String, Option<u32>)> {
    let token = token.trim().trim_matches(|c| c == '(' || c == ')' || c == '"' || c == '\'');
    let mut parts: Vec<&str> = token.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    // Pop the trailing run of numeric components (`:line:col`); the line is
    // the FIRST of that run, the rest are columns.
    let mut nums: Vec<u32> = Vec::new();
    while parts.len() > 1 {
        match parse_u32(parts[parts.len() - 1]) {
            Some(n) => {
                nums.push(n);
                parts.pop();
            }
            None => break,
        }
    }
    if nums.is_empty() {
        return None;
    }
    let line = nums.last().copied();
    let path = parts.join(":");
    // Windows drive letter (`C:\…`) rejoins correctly since `C` is not numeric.
    if !looks_like_code_path(&path) && !looks_like_code_path(token) {
        return None;
    }
    Some((path, line))
}

/// Parse combined test stdout+stderr into failures (deduped, capped at 20).
pub fn parse_failures(output: &str) -> Vec<Failure> {
    let mut out: Vec<Failure> = Vec::new();
    let mut seen: std::collections::HashSet<(String, u32)> = std::collections::HashSet::new();
    let push = |kind: &str, file: Option<String>, line: Option<u32>, msg: &str,
                    out: &mut Vec<Failure>, seen: &mut std::collections::HashSet<(String, u32)>| {
        let key = (file.clone().unwrap_or_default(), line.unwrap_or(0));
        if seen.insert(key) {
            out.push(Failure::new(kind, file, line, msg.to_string()));
        }
    };

    let lines: Vec<&str> = output.lines().collect();
    // Python: remember the trailing error header (`ValueError: ...`).
    let mut py_error: Option<String> = None;
    for line in &lines {
        let t = line.trim();
        if t.is_empty() || t == "^" || t.starts_with("^^") {
            continue;
        }
        if let Some(first) = t.split(':').next() {
            let head = first.trim();
            if head.ends_with("Error")
                || head.ends_with("Exception")
                || head.ends_with("Failure")
                || head == "E"
                || t.starts_with("FAILED")
                || t.starts_with("ERROR ")
            {
                py_error = Some(t.to_string());
            }
        }
    }
    // Rust: nearest `error…` header above a `-->` location.
    let mut last_rust_error: Option<String> = None;

    for (i, line) in lines.iter().enumerate() {
        if out.len() >= 20 {
            break;
        }
        let t = line.trim();

        // 1) Rust panic: thread 'x' panicked at src/main.rs:12:5:
        if let Some(at) = t.split("panicked at ").nth(1) {
            let loc = at.trim_end_matches(':').split_whitespace().next().unwrap_or("");
            if let Some((f, l)) = split_file_line(loc) {
                push("panic", Some(f), l, t, &mut out, &mut seen);
                continue;
            }
        }
        // Track `error[…]: msg` headers for the next `-->` line.
        if t.starts_with("error") && (t.contains(':') || t.len() > 6) {
            last_rust_error = Some(t.to_string());
        }
        // 2) rustc location: --> src/main.rs:12:5
        if let Some(loc) = t.strip_prefix("--> ") {
            let loc = loc.split_whitespace().next().unwrap_or("");
            if let Some((f, l)) = split_file_line(loc) {
                let msg = last_rust_error.clone().unwrap_or_else(|| format!("compile error at {}", loc));
                push("compile", Some(f), l, &msg, &mut out, &mut seen);
                continue;
            }
        }
        // 3) Python traceback frame: File "/x/y.py", line 12, in fn
        if t.starts_with("File \"") {
            if let Some(q1) = t.find('"') {
                if let Some(q2) = t[q1 + 1..].find('"') {
                    let path = t[q1 + 1..q1 + 1 + q2].to_string();
                    let line_no = t[q1 + 1 + q2..]
                        .split("line ")
                        .nth(1)
                        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                        .and_then(parse_u32);
                    let msg = py_error.clone().unwrap_or_else(|| "traceback frame".to_string());
                    push("traceback", Some(path), line_no, &msg, &mut out, &mut seen);
                    continue;
                }
            }
        }
        // 4) Node stack: at fn (/x/y.js:10:15) or at /x/y.js:10:15
        let stripped = t.strip_prefix("at ").unwrap_or("");
        if !stripped.is_empty() {
            let candidate = stripped
                .rfind('(')
                .map(|p| &stripped[p + 1..])
                .unwrap_or(stripped)
                .trim_end_matches(')');
            if candidate.contains(':') && candidate.chars().any(|c| c.is_ascii_digit()) {
                if let Some((f, l)) = split_file_line(candidate) {
                    // Skip node internals.
                    if !f.starts_with("node:") {
                        push("stack", Some(f), l, t, &mut out, &mut seen);
                        continue;
                    }
                }
            }
        }
        // 5) Test-result headers: FAILED / FAIL / AssertionError lines.
        if t == "FAILED"
            || t == "FAIL"
            || t.starts_with("FAILED ")
            || t.starts_with("FAIL ")
            || t.starts_with("● ")
        {
            push("test", None, None, t, &mut out, &mut seen);
            continue;
        }
        if t.starts_with("---- ") && t.ends_with(" stdout ----") {
            let name = t.trim_start_matches("---- ").trim_end_matches(" stdout ----");
            push("test", None, None, &format!("failing test: {}", name), &mut out, &mut seen);
            continue;
        }
        // 6) Generic file:line fallback (also catches assertion contexts).
        if i > 0 && t.len() < 260 {
            for token in t.split_whitespace() {
                if token.matches(':').count() >= 1 && token.chars().any(|c| c.is_ascii_digit()) {
                    if let Some((f, l)) = split_file_line(token) {
                        // Require the line to look like more than a bare path.
                        push("location", Some(f), l, t, &mut out, &mut seen);
                        break;
                    }
                }
            }
        }
    }
    out
}

/// Last `n` lines of output for prompt context / display.
pub fn tail_lines(output: &str, n: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}
