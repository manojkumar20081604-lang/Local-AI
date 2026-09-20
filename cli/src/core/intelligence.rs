// Port of frontend/src/services/projectIntelligence.ts to Rust + hybrid embeddings
use super::embeddings::{cosine, Embedder};
use super::fs::ProjectFile;
use super::index::ProjectIndex;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectIntent {
    General,
    Explain,
    Debug,
    Find,
    Architecture,
    Edit,
    Review,
    Run,
    Unknown,
}

impl std::fmt::Display for ProjectIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::General => "general",
            Self::Explain => "explain",
            Self::Debug => "debug",
            Self::Find => "find",
            Self::Architecture => "architecture",
            Self::Edit => "edit",
            Self::Review => "review",
            Self::Run => "run",
            Self::Unknown => "unknown",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone)]
pub struct IntentResult {
    pub intent: ProjectIntent,
    pub confidence: f32,
    pub keywords: Vec<String>,
}

fn intent_rules() -> Vec<(ProjectIntent, Vec<&'static str>)> {
    vec![
        (ProjectIntent::Debug, vec!["error","bug","broken","crash","fails","failed","failure","exception","not working","doesn't work","doesnt work","issue","problem","fix"]),
        (ProjectIntent::Edit, vec!["change","modify","update","edit","replace","add","remove","delete","implement","make it","write","create"]),
        (ProjectIntent::Explain, vec!["explain","what does","what is","how does","why does","understand","meaning","describe"]),
        (ProjectIntent::Find, vec!["find","where","which file","locate","search","show me","look for"]),
        (ProjectIntent::Architecture, vec!["architecture","structure","project structure","how is the project","components","dependencies","flow","frontend","backend","database","api"]),
        (ProjectIntent::Review, vec!["review","audit","improve","improvements","quality","clean","refactor","optimize","optimization"]),
        (ProjectIntent::Run, vec!["run","build","compile","test","start","launch","command","terminal"]),
    ]
}

pub fn detect_intent(text: &str) -> IntentResult {
    let normalized = text.to_lowercase().trim().to_string();
    if normalized.is_empty() {
        return IntentResult { intent: ProjectIntent::Unknown, confidence: 0.0, keywords: vec![] };
    }
    let mut best = ProjectIntent::General;
    let mut best_score = 0usize;
    let mut matched = Vec::new();
    for (intent, keywords) in intent_rules() {
        let matches: Vec<String> = keywords.into_iter().filter(|k| normalized.contains(*k)).map(|s| s.to_string()).collect();
        if matches.len() > best_score {
            best_score = matches.len();
            best = intent;
            matched = matches;
        }
    }
    if best_score == 0 {
        return IntentResult { intent: ProjectIntent::General, confidence: 0.2, keywords: vec![] };
    }
    let confidence = (0.45 + best_score as f32 * 0.15).min(0.95);
    IntentResult { intent: best, confidence, keywords: matched }
}

pub fn intent_instructions(intent: &ProjectIntent) -> &'static str {
    match intent {
        ProjectIntent::Debug => "Focus on actual errors, relevant source files, imports, data flow, likely root cause, concrete fixes. Do not invent errors.",
        ProjectIntent::Edit => "Identify relevant files, understand existing implementation, preserve behavior, mention exact paths.",
        ProjectIntent::Explain => "Use actual project files as source of truth, explain with exact paths and functions, do not invent structure.",
        ProjectIntent::Find => "Identify most relevant files/paths, use only provided context.",
        ProjectIntent::Architecture => "Analyze directories, source files, frontend/backend boundaries, data flow, services.",
        ProjectIntent::Review => "Separate facts, problems/risks, suggested improvements, mention exact paths.",
        ProjectIntent::Run => "Use actual files/config, do not invent commands.",
        _ => "Answer using provided project context, prefer facts over assumptions.",
    }
}

#[derive(Debug, Clone)]
pub struct RelevantFile {
    pub file: ProjectFile,
    pub score: f32,
    pub reasons: Vec<String>,
}

fn is_ignored(path_parts: &[String], name_lower: &str) -> bool {
    let ignored = ["node_modules",".git","dist","build","target",".next",".vite","coverage",".cache","__pycache__",".venv","venv",".idea",".vscode"];
    if path_parts.iter().any(|p| ignored.contains(&p.as_str())) { return true; }
    matches!(name_lower, "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "cargo.lock")
}

fn tokenize(text: &str) -> Vec<String> {
    let stop = ["the","and","for","that","this","with","from","what","why","how","does","is","are","was","not","working","work","please","can","you","my","project","file","code","make","it"];
    let lower = text.to_lowercase();
    let cleaned: String = lower.chars().map(|c| if c.is_alphanumeric() || c=='.' || c=='/' || c=='_' || c=='-' { c } else { ' ' }).collect();
    let mut uniq = std::collections::HashSet::new();
    for tok in cleaned.split_whitespace() {
        if tok.len() >= 2 && !stop.contains(&tok) {
            uniq.insert(tok.to_string());
        }
    }
    uniq.into_iter().collect()
}

fn ext_weight(ext: &str) -> f32 {
    match ext {
        ".ts" => 0.12, ".tsx" => 0.14, ".js" => 0.10, ".jsx" => 0.12, ".css" => 0.07, ".scss" => 0.06,
        ".html" => 0.06, ".json" => 0.05, ".rs" => 0.14, ".py" => 0.12, ".go" => 0.11, ".java" => 0.10,
        _ => 0.02,
    }
}

pub fn rank_relevant_files(text: &str, files: &[ProjectFile], intent: &IntentResult) -> Vec<RelevantFile> {
    let tokens = tokenize(text);
    let mut scored = Vec::new();
    for file in files.iter().filter(|f| !f.is_directory) {
        let path_lower = file.path.to_lowercase();
        let name_lower = file.name.to_lowercase();
        let parts: Vec<String> = path_lower.split('/').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
        if is_ignored(&parts, &name_lower) {
            continue;
        }
        let mut score = 0.0;
        let mut reasons = Vec::new();
        let core = ["app.tsx","app.ts","main.tsx","main.ts","index.tsx","index.ts","main.rs","lib.rs","package.json","vite.config.ts","tsconfig.json"];
        if core.contains(&name_lower.as_str()) {
            score += 0.18; reasons.push("core project file".to_string());
        }
        let exact = tokens.iter().any(|t| name_lower == *t || name_lower.trim_end_matches(|c: char| !c.is_alphabetic()).contains(t) || path_lower.contains(t));
        if exact {
            score += 0.55; reasons.push("query/path match".to_string());
        }
        let matches: Vec<String> = tokens.iter().filter(|t| name_lower.contains(t.as_str()) || path_lower.contains(t.as_str())).cloned().collect();
        if !matches.is_empty() {
            score += (matches.len() as f32 * 0.08).min(0.25);
            reasons.push(format!("keyword match: {}", matches.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
        }
        let ext = std::path::Path::new(&path_lower).extension().and_then(|e| e.to_str()).map(|e| format!(".{}", e)).unwrap_or_default();
        score += ext_weight(&ext);
        if path_lower.contains("src/") || path_lower.contains("app/") || path_lower.contains("components/") {
            score += 0.06; reasons.push("source directory".to_string());
        }
        if intent.intent == ProjectIntent::Debug && [".ts",".tsx",".js",".jsx",".rs",".py",".go"].contains(&ext.as_str()) {
            score += 0.04; reasons.push("debuggable source".to_string());
        }
        if intent.intent == ProjectIntent::Architecture && (name_lower.contains("config") || name_lower.contains("main")) {
            score += 0.05; reasons.push("architecture-relevant".to_string());
        }
        if score > 0.0 {
            scored.push(RelevantFile { file: file.clone(), score: score.min(1.0), reasons });
        }
    }
    scored.sort_by(|a,b| b.score.partial_cmp(&a.score).unwrap());
    scored
}

pub fn select_relevant_files(text: &str, files: &[ProjectFile], limit: usize) -> Vec<RelevantFile> {
    let intent = detect_intent(text);
    rank_relevant_files(text, files, &intent).into_iter().filter(|r| r.score >= 0.10).take(limit).collect()
}

pub fn build_project_context(text: &str, files: &[ProjectFile], file_loader: impl Fn(&str) -> Option<String>) -> String {
    let intent = detect_intent(text);
    let relevant = select_relevant_files(text, files, 6);
    let fallback: Vec<&ProjectFile> = files.iter().filter(|f| !f.is_directory).take(6).collect();
    let to_use = if relevant.is_empty() { fallback } else { relevant.iter().map(|r| &r.file).collect() };
    if to_use.is_empty() { return String::new(); }
    let mut contexts = Vec::new();
    for f in to_use {
        if let Some(content) = file_loader(&f.path) {
            let limited = if content.len() > 8000 { content[..8000].to_string() } else { content };
            contexts.push(format!("FILE: {}\n\n{}", f.path, limited));
        }
    }
    if contexts.is_empty() { return String::new(); }
    // Authoritative header
    let inventory: Vec<String> = files.iter().filter(|f| !f.is_directory).take(100).map(|f| format!("- {} ({} chars)", f.path, 0)).collect();
    let header = if files.len() > 0 {
        format!("AUTHORITATIVE REAL PROJECT FILES (verified via `files list`, DO NOT invent outside this set, total {} files):\n{}\n", files.len(), inventory.join("\n"))
    } else { String::new() };
    format!(
        "{header}PROJECT INTENT: {}\nCONFIDENCE: {:.2}\n{}\n\nUse REAL project files as context:\n\n{}",
        intent.intent, intent.confidence, intent_instructions(&intent.intent), contexts.join("\n\n==============================\n\n")
    )
}

// ---------- Hybrid embedding ranker (Phase 2) ----------

pub fn hybrid_rank(
    text: &str,
    files: &[ProjectFile],
    index: Option<&ProjectIndex>,
    embedder: Option<Arc<dyn Embedder>>,
) -> Vec<RelevantFile> {
    let intent = detect_intent(text);
    // Try embedding path if both index and embedder available
    if let (Some(idx), Some(emb)) = (index, embedder) {
        if let Ok(query_embs) = emb.embed(&[text.to_string()]) {
            if let Some(qemb) = query_embs.first() {
                let scored = crate::core::index::query_index(idx, qemb, 20);
                // Convert to RelevantFile with hybrid score: 0.55 cosine + 0.25 keyword + rest
                let keyword_map = keyword_scores(text, files, &intent);
                let mut hybrid: Vec<RelevantFile> = Vec::new();
                for (entry, cos, _chunk_idx) in scored {
                    if let Some(pf) = files.iter().find(|f| f.path == entry.path) {
                        let kw = keyword_map.get(&pf.path).cloned().unwrap_or(0.0);
                        // cosine 0..1, keyword 0..0.8
                        let score = (0.55 * cos + 0.35 * kw + 0.10).min(1.0);
                        let mut reasons = vec![format!("cosine {:.2}", cos)];
                        if kw > 0.1 { reasons.push(format!("keyword {:.2}", kw)); }
                        if pf.path.contains("src/") { reasons.push("source".into()); }
                        hybrid.push(RelevantFile { file: pf.clone(), score, reasons });
                    }
                }
                // Also include keyword-only files that may have been missed (cosine < threshold but keyword high)
                let mut seen: std::collections::HashSet<String> = hybrid.iter().map(|r| r.file.path.clone()).collect();
                for rf in rank_relevant_files(text, files, &intent) {
                    if !seen.contains(&rf.file.path) && rf.score >= 0.45 {
                        seen.insert(rf.file.path.clone());
                        hybrid.push(rf);
                    }
                }
                hybrid.sort_by(|a,b| b.score.partial_cmp(&a.score).unwrap());
                return hybrid.into_iter().filter(|r| r.score >= 0.25).take(10).collect();
            }
        }
    }
    // Fallback to keyword
    rank_relevant_files(text, files, &intent)
}

fn keyword_scores(text: &str, files: &[ProjectFile], intent: &IntentResult) -> std::collections::HashMap<String, f32> {
    let mut map = std::collections::HashMap::new();
    for rf in rank_relevant_files(text, files, intent) {
        map.insert(rf.file.path.clone(), rf.score);
    }
    map
}

pub fn build_project_context_hybrid(
    text: &str,
    files: &[ProjectFile],
    index: Option<&ProjectIndex>,
    embedder: Option<Arc<dyn Embedder>>,
    file_loader: impl Fn(&str) -> Option<String>,
) -> String {
    let intent = detect_intent(text);
    let hybrid = hybrid_rank(text, files, index, embedder);
    let relevant = hybrid.into_iter().filter(|r| r.score >= 0.25).take(6).collect::<Vec<_>>();
    let fallback: Vec<&ProjectFile> = files.iter().filter(|f| !f.is_directory).take(6).collect();
    let to_use: Vec<&ProjectFile> = if relevant.is_empty() { fallback } else { relevant.iter().map(|r| &r.file).collect() };
    if to_use.is_empty() { return String::new(); }
    let mut contexts = Vec::new();
    for f in to_use {
        if let Some(content) = file_loader(&f.path) {
            let limited = if content.len() > 8000 { content[..8000].to_string() } else { content };
            contexts.push(format!("FILE: {}\n\n{}", f.path, limited));
        }
    }
    if contexts.is_empty() { return String::new(); }
    // Build authoritative inventory header with real file list
    let inventory_lines: Vec<String> = files.iter().filter(|f| !f.is_directory).map(|f| format!("- {}", f.path)).take(80).collect();
    let header = format!(
        "AUTHORITATIVE REAL PROJECT FILES (verified via `files list`, total {} files — DO NOT invent outside this set):\n{}\n{} truncated, see inventory above.\n",
        files.len(),
        inventory_lines.join("\n"),
        if files.len() > 80 { format!("(+{} more)", files.len() - 80) } else { String::new() }
    );
    format!(
        "{header}PROJECT INTENT: {}\nCONFIDENCE: {:.2}\n{}\n\nUse REAL project files as context:\n\n{}",
        intent.intent, intent.confidence, intent_instructions(&intent.intent), contexts.join("\n\n==============================\n\n")
    )
}
