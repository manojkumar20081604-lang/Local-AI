//! Code graph — RAG 2.0 symbols + dependency edges (Phase 3.2).
//!
//! The plan calls for tree-sitter (Rust, TS/JS, Python). To keep the binary
//! dependency-free and offline, this module implements a lightweight
//! line-oriented parser with the same output shape:
//!
//! `{file, lang, defs[{name, kind, line}], imports[]}`
//!
//! It is intentionally conservative (misses exotic syntax, never invents):
//! every def/import cites a real `line`, and the graph only feeds ranking —
//! the verifier still rejects invented files/symbols downstream.
//!
//! [`CodeGraph::imported_by`] answers "who imports X?", [`resolve_query`]
//! answers `"where is auth handled?"` via symbol names instead of raw text.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::fs::ProjectFile;

/// One definition: `fn`, `struct`, `class`, `interface`, …
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDef {
    pub name: String,
    pub kind: String,
    pub line: u32,
}

/// All symbols for one file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSymbols {
    pub file: String,
    pub lang: String,
    pub defs: Vec<SymbolDef>,
    pub imports: Vec<String>,
}

/// Directed import edge `from → to` (both repo-relative paths when known).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEdge {
    pub from: String,
    pub to: String,
    pub raw: String,
}

/// Whole-project graph: per-file symbols + resolved edges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeGraph {
    pub files: Vec<FileSymbols>,
    pub edges: Vec<ImportEdge>,
}

impl CodeGraph {
    /// Files that import `path` (direct importers).
    pub fn imported_by(&self, path: &str) -> Vec<String> {
        self.edges
            .iter()
            .filter(|e| e.to == path)
            .map(|e| e.from.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Direct imports of `path`.
    pub fn imports_of(&self, path: &str) -> Vec<String> {
        self.files
            .iter()
            .find(|f| f.file == path)
            .map(|f| f.imports.clone())
            .unwrap_or_default()
    }

    /// Defs for one file.
    pub fn defs_of(&self, path: &str) -> Vec<SymbolDef> {
        self.files
            .iter()
            .find(|f| f.file == path)
            .map(|f| f.defs.clone())
            .unwrap_or_default()
    }
}

fn lang_of(path: &str) -> &'static str {
    if path.ends_with(".rs") {
        "rust"
    } else if path.ends_with(".ts") || path.ends_with(".tsx") {
        "ts"
    } else if path.ends_with(".js") || path.ends_with(".jsx") || path.ends_with(".mjs") || path.ends_with(".cjs") {
        "js"
    } else if path.ends_with(".py") {
        "python"
    } else if path.ends_with(".go") {
        "go"
    } else {
        "other"
    }
}

/// Parse one file's text into symbols. `None` for unsupported languages.
pub fn parse_file(path: &str, content: &str) -> Option<FileSymbols> {
    let lang = lang_of(path);
    if lang == "other" {
        return None;
    }
    let mut defs = Vec::new();
    let mut imports = Vec::new();
    for (idx, raw_line) in content.lines().enumerate() {
        let line = (idx + 1) as u32;
        let t = raw_line.trim();
        if t.is_empty() || t.starts_with("//") && lang != "python" || t.starts_with('#') && lang == "rust" {
            // `#…` in Rust is an attribute — the def is on the next line, skip noise.
            if lang == "rust" && t.starts_with('#') {
                continue;
            }
            if t.starts_with("//") || (lang == "python" && t.starts_with('#')) {
                continue;
            }
        }
        match lang {
            "rust" => parse_rust_line(t, line, &mut defs, &mut imports),
            "ts" | "js" => parse_ts_line(t, line, &mut defs, &mut imports),
            "python" => parse_py_line(t, line, &mut defs, &mut imports),
            "go" => parse_go_line(t, line, &mut defs, &mut imports),
            _ => {}
        }
    }
    Some(FileSymbols { file: path.to_string(), lang: lang.to_string(), defs, imports })
}

fn ident_token(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' )
        .collect()
}

fn parse_rust_line(t: &str, line: u32, defs: &mut Vec<SymbolDef>, imports: &mut Vec<String>) {
    // `use crate::foo::bar;` / `use foo::{A, B};` — keep the raw module path.
    if let Some(rest) = t.strip_prefix("use ") {
        let raw = rest.trim_end_matches(';').split("::{").next().unwrap_or(rest).trim().trim_end_matches(';').to_string();
        if !raw.is_empty() {
            imports.push(raw);
        }
        return;
    }
    // `mod foo;` — local module edge.
    if let Some(rest) = t.strip_prefix("mod ") {
        let name = ident_token(rest.trim());
        if !name.is_empty() {
            imports.push(format!("mod:{}", name));
        }
        return;
    }
    let kinds = [
        ("pub fn ", "fn"), ("pub async fn ", "fn"), ("async fn ", "fn"), ("fn ", "fn"),
        ("pub struct ", "struct"), ("struct ", "struct"),
        ("pub enum ", "enum"), ("enum ", "enum"),
        ("pub trait ", "trait"), ("trait ", "trait"),
        ("pub mod ", "mod"), ("pub const ", "const"), ("pub static ", "static"),
    ];
    for (prefix, kind) in kinds {
        if let Some(rest) = t.strip_prefix(prefix) {
            let name = ident_token(rest.trim_start());
            if !name.is_empty() && name != "main" || (name == "main" && kind == "fn") {
                // `main` is a real def — keep it (useful for "where is main?").
                defs.push(SymbolDef { name, kind: kind.to_string(), line });
            }
            return;
        }
    }
    // `impl Foo` — record the type being implemented.
    if let Some(rest) = t.strip_prefix("impl ") {
        let name = ident_token(rest.split(['<', ' ', '{']).next().unwrap_or("").trim());
        if !name.is_empty() {
            defs.push(SymbolDef { name, kind: "impl".to_string(), line });
        }
    }
}

fn parse_ts_line(t: &str, line: u32, defs: &mut Vec<SymbolDef>, imports: &mut Vec<String>) {
    // `import … from 'path'` / `import 'path'`
    if t.starts_with("import ") {
        if let Some(from_pos) = t.find(" from ") {
            let rhs = t[from_pos + 6..].trim().trim_matches(|c| c == '\'' || c == '"' || c == ';').trim();
            if !rhs.is_empty() {
                imports.push(rhs.to_string());
            }
        } else {
            // Side-effect import: `import './x';`
            let quoted = t.trim_start_matches("import").trim().trim_matches(|c| c == '\'' || c == '"' || c == ';').trim();
            if !quoted.is_empty() && (quoted.contains('/') || quoted.contains('.')) {
                imports.push(quoted.to_string());
            }
        }
        // `import { AuthService } …` — also a def-like reference for ranking.
        return;
    }
    if t.starts_with("export ") {
        let rest = t.trim_start_matches("export").trim_start_matches("default").trim();
        push_ts_def(rest, line, defs);
        return;
    }
    // `const X = require('path')`
    if t.contains("require(") {
        if let Some(q1) = t.find("require(") {
            let after = &t[q1 + 8..];
            let q = after.trim_start().trim_start_matches(['\'', '"']);
            let path: String = q.chars().take_while(|c| !['\'', '"'].contains(c)).collect();
            if !path.is_empty() {
                imports.push(path);
            }
        }
    }
    push_ts_def(t, line, defs);
}

fn push_ts_def(t: &str, line: u32, defs: &mut Vec<SymbolDef>) {
    let t = t.trim_start_matches("async ").trim();
    for (prefix, kind) in [
        ("function ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("type ", "type"),
        ("enum ", "enum"),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let name = ident_token(rest.trim_start());
            if !name.is_empty() {
                defs.push(SymbolDef { name, kind: kind.to_string(), line });
            }
            return;
        }
    }
    // `const AuthService = …` / `let x =` at top level — record const names.
    if t.starts_with("const ") || t.starts_with("let ") || t.starts_with("var ") {
        let rest = t.split_whitespace().nth(1).unwrap_or("");
        let name = ident_token(rest);
        if !name.is_empty() && name.chars().next().map(|c| c.is_uppercase() || name.len() > 1).unwrap_or(false) {
            defs.push(SymbolDef { name, kind: "const".to_string(), line });
        }
    }
}

fn parse_py_line(t: &str, line: u32, defs: &mut Vec<SymbolDef>, imports: &mut Vec<String>) {
    if let Some(rest) = t.strip_prefix("import ") {
        for part in rest.split(',') {
            let name = part.split_whitespace().next().unwrap_or("").trim();
            if !name.is_empty() {
                imports.push(name.to_string());
            }
        }
        return;
    }
    if let Some(rest) = t.strip_prefix("from ") {
        if let Some(mod_part) = rest.split(" import ").next() {
            let m = mod_part.trim();
            if !m.is_empty() {
                imports.push(m.to_string());
            }
        }
        return;
    }
    if let Some(rest) = t.strip_prefix("def ") {
        let name = ident_token(rest);
        if !name.is_empty() {
            defs.push(SymbolDef { name, kind: "fn".to_string(), line });
        }
        return;
    }
    if let Some(rest) = t.strip_prefix("class ") {
        let name = ident_token(rest);
        if !name.is_empty() {
            defs.push(SymbolDef { name, kind: "class".to_string(), line });
        }
    }
    // `async def …`
    if let Some(rest) = t.strip_prefix("async def ") {
        let name = ident_token(rest);
        if !name.is_empty() {
            defs.push(SymbolDef { name, kind: "fn".to_string(), line });
        }
    }
}

fn parse_go_line(t: &str, line: u32, defs: &mut Vec<SymbolDef>, imports: &mut Vec<String>) {
    if t.starts_with("import ") {
        let quoted: String = t.chars().filter(|c| *c != '"' && *c != '\'' && *c != '(').collect();
        let tail = quoted.trim_start_matches("import").trim();
        if !tail.is_empty() {
            imports.push(tail.to_string());
        }
        return;
    }
    if let Some(rest) = t.strip_prefix("func ") {
        // `func (r *T) Name(` or `func Name(`
        let rest = rest.trim();
        let name_part = if rest.starts_with('(') {
            rest.find(')').map(|p| rest[p + 1..].trim()).unwrap_or("")
        } else {
            rest
        };
        let name = ident_token(name_part);
        if !name.is_empty() {
            defs.push(SymbolDef { name, kind: "fn".to_string(), line });
        }
        return;
    }
    if let Some(rest) = t.strip_prefix("type ") {
        let name = ident_token(rest.trim());
        if !name.is_empty() {
            defs.push(SymbolDef { name, kind: "type".to_string(), line });
        }
    }
}

/// Resolve a raw import string to a repo-relative file when possible.
///
/// Handles `./x`, `../x`, `@/x`, `src/x` and bare `mod:name` (Rust sibling).
/// Returns `None` when the target cannot be mapped (external crate/package).
pub fn resolve_import(from_file: &str, raw: &str, all_paths: &HashSet<String>) -> Option<String> {
    if raw.starts_with("mod:") {
        // Rust sibling module: `src/auth.rs` + `mod:foo` → `src/foo.rs`.
        let name = raw.trim_start_matches("mod:");
        let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for cand in [format!("{}/{}.rs", dir, name), format!("{}/{}/mod.rs", dir, name)] {
            let cand = cand.trim_start_matches('/').to_string();
            if all_paths.contains(&cand) {
                return Some(cand);
            }
        }
        return None;
    }
    // Skip std/external: `std::…`, `core::`, `crate::` roots handled below.
    if raw.starts_with("std")
        || raw.starts_with("core::")
        || raw.starts_with("alloc::")
        || ["react", "react-dom", "fs", "path", "os", "sys", "tokio", "serde", "anyhow", "clap"]
            .iter()
            .any(|ext| raw == *ext || raw.starts_with(&format!("{}::", ext)))
    {
        return None;
    }
    // `crate::foo::bar` → try `src/foo/bar.rs`, `src/foo.rs`, …
    if let Some(rest) = raw.strip_prefix("crate::") {
        let rel = rest.replace("::", "/");
        for cand in [
            format!("src/{}.rs", rel),
            format!("src/{}/mod.rs", rel),
            format!("{}.rs", rel),
        ] {
            if all_paths.contains(&cand) {
                return Some(cand);
            }
        }
        return None;
    }
    // Relative / aliased TS imports: `./auth`, `../routes/auth`, `@/services/x`.
    let mut norm = raw.trim().to_string();
    for prefix in ["@/", "~/"] {
        if let Some(stripped) = norm.strip_prefix(prefix) {
            norm = format!("src/{}", stripped);
            break;
        }
    }
    if norm.starts_with('.') {
        let dir = from_file.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
        for seg in norm.split('/') {
            match seg {
                "." | "" => {}
                ".." => {
                    parts.pop();
                }
                s => parts.push(s),
            }
        }
        norm = parts.join("/");
    }
    // Try with known extensions.
    let mut candidates = vec![norm.clone()];
    for ext in [".ts", ".tsx", ".js", ".jsx", ".py", ".rs", ".go"] {
        if !norm.ends_with(ext) {
            candidates.push(format!("{}{}", norm, ext));
        }
    }
    candidates.push(format!("{}/index.ts", norm));
    candidates.push(format!("{}/mod.rs", norm));
    candidates.push(format!("{}/__init__.py", norm));
    for cand in candidates {
        let cand = cand.trim_start_matches('/').trim_start_matches("./").to_string();
        if all_paths.contains(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Build the whole-project graph from file contents.
pub fn build_graph(files: &[ProjectFile], loader: impl Fn(&str) -> Option<String>) -> CodeGraph {
    let mut parsed: Vec<FileSymbols> = Vec::new();
    for f in files.iter().filter(|f| !f.is_directory) {
        if lang_of(&f.path) == "other" {
            continue;
        }
        if let Some(content) = loader(&f.path) {
            if content.len() > 500_000 || content.chars().any(|c| c == '\0') {
                continue;
            }
            if let Some(sym) = parse_file(&f.path, &content) {
                parsed.push(sym);
            }
        }
    }
    let all_paths: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let mut edges = Vec::new();
    for fsym in &parsed {
        for raw in &fsym.imports {
            if let Some(to) = resolve_import(&fsym.file, raw, &all_paths) {
                edges.push(ImportEdge { from: fsym.file.clone(), to, raw: raw.clone() });
            }
        }
    }
    CodeGraph { files: parsed, edges }
}

/// Symbol-match score for one file: does the query mention a def/import name?
/// Returns `0.0–1.0` (1.0 = exact symbol token match).
pub fn symbol_match_score(query: &str, fsym: &FileSymbols) -> f32 {
    let tokens: Vec<String> = query
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|t| t.len() >= 3)
        .map(|s| s.to_string())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let mut best = 0.0f32;
    for tok in &tokens {
        for d in &fsym.defs {
            let name = d.name.to_lowercase();
            if name == *tok {
                best = best.max(1.0);
            } else if name.contains(tok) || tok.contains(&name as &str) {
                best = best.max(0.6);
            }
        }
        for imp in &fsym.imports {
            if imp.to_lowercase().contains(tok) {
                best = best.max(0.4);
            }
        }
        // File-stem match (auth in src/auth.rs).
        if let Some(stem) = fsym.file.rsplit('/').next().and_then(|s| s.split('.').next()) {
            if stem.to_lowercase() == *tok {
                best = best.max(0.7);
            }
        }
    }
    best
}

/// Resolve `"where is auth handled?"` via the graph: rank files whose defs or
/// stems match the query tokens. Falls back to empty (caller uses text rank).
pub fn resolve_query(query: &str, graph: &CodeGraph) -> Vec<(String, String)> {
    let mut scored: Vec<(String, String, f32)> = Vec::new();
    for fsym in &graph.files {
        let s = symbol_match_score(query, fsym);
        if s > 0.0 {
            let chain = fsym
                .defs
                .iter()
                .take(3)
                .map(|d| format!("{}()", d.name))
                .collect::<Vec<_>>()
                .join(" → ");
            scored.push((fsym.file.clone(), chain, s));
        }
    }
    scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(8).map(|(f, c, _)| (f, c)).collect()
}

/// Git recency per file: `1.0` for files in the latest commit, decaying by
/// commit depth. Empty when not a git repo (caller treats as 0.0).
pub fn git_recency(project_root: Option<&std::path::Path>) -> HashMap<String, f32> {
    let mut map = HashMap::new();
    let Some(root) = project_root else { return map };
    // `git log --name-only --format=` lists touched files newest-first.
    let out = std::process::Command::new("git")
        .args(["log", "--name-only", "--format=%H", "-20"])
        .current_dir(root)
        .output();
    let Ok(out) = out else { return map };
    if !out.status.success() {
        return map;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut depth = 0u32;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.len() == 40 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            depth += 1;
            continue;
        }
        // File line — first touch wins (newest).
        map.entry(t.to_string()).or_insert_with(|| match depth {
            1 => 1.0,
            2 => 0.7,
            3 => 0.5,
            4..=6 => 0.3,
            _ => 0.15,
        });
    }
    map
}
