use std::collections::{HashMap, HashSet};
use super::fs::ProjectFile;
use super::projects::Project;
use super::fs as core_fs;

/// Result of verifying a single claimed file path
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileVerification {
    pub claimed: String,
    pub exists: bool,
    pub kind: String, // "file" | "dir" | "invented"
}

/// Overall verifier report
#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifierReport {
    pub mentioned_files: Vec<FileVerification>,
    pub invented_files: Vec<String>,
    pub verified_files: Vec<String>,
    pub total_mentioned: usize,
    pub invented_count: usize,
    pub hallucination_score: f32, // invented / total
    pub is_hallucinated: bool, // score > 0.3 or invented >0 in strict
    pub grounding_mode: String,
    pub symbol_checks: Vec<SymbolCheck>,
    pub edit_block_errors: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SymbolCheck {
    pub symbol: String,
    pub found: bool,
    pub files: Vec<String>, // where found
}

/// Extract file path mentions from text.
/// Strategies:
/// 1. Regex for `path/to/file.ext` (with / and .)
/// 2. `<FILE>` or `FILE:` blocks
/// 3. Markdown code spans `src/...`
pub fn extract_file_mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    // Helper to add normalized path
    let mut add = |p: String| {
        let mut s = p.trim().trim_matches(|c| c == '`' || c == '"' || c == '\'' || c == '*' || c == '[' || c == ']').to_string();
        // Strip trailing punctuation
        while s.ends_with('.') || s.ends_with(',') || s.ends_with(')') || s.ends_with(']') {
            s.pop();
        }
        // Must look like a file path: contains / or . and not just a word
        if s.len() < 3 || s.len() > 200 { return; }
        // Reject obvious non-paths like "http://"
        if s.starts_with("http://") || s.starts_with("https://") { return; }
        // Must contain / or .ext
        let has_slash = s.contains('/');
        let has_dot = s.contains('.') && s.rsplit('.').next().map(|ext| ext.len() >= 1 && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or(false);
        if !has_slash && !has_dot { return; }
        // Reject if it's just a sentence
        if s.contains(' ') { return; }
        // Normalize: remove leading ./
        if s.starts_with("./") { s = s[2..].to_string(); }
        // Skip if already seen
        if seen.contains(&s) { return; }
        seen.insert(s.clone());
        out.push(s);
    };

    // 1. Scan for <FILE> blocks
    for block in extract_blocks(text, "FILE:") {
        // block is like "src/foo.rs\nCONTENT:" → take first line/token
        let first = block.split_whitespace().next().unwrap_or("").split("CONTENT:").next().unwrap_or("").trim();
        if !first.is_empty() { add(first.to_string()); }
    }
    for block in extract_blocks(text, "<CREATE_FILE>") {
        // inside: FILE: path
        if let Some(pos) = block.find("FILE:") {
            let after = &block[pos+5..];
            let path = after.split("CONTENT:").next().unwrap_or("").split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() { add(path.to_string()); }
        }
    }
    // 2. Regex-like scan for path patterns: word chars / . - _ (including backticks)
    let tokens: Vec<String> = text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == ';' || c == '"' || c == '\'' || c == '<' || c == '>' || c == '[' || c == ']' || c == '`').map(|s| s.to_string()).collect();
    for tok in tokens {
        // Clean token: remove trailing : etc and leading/trailing backticks already split, but also handle colon
        let t = tok.trim().trim_end_matches(|c| c == ':' || c == '.' || c == ',' || c == ';').to_string();
        if t.is_empty() { continue; }
        // Skip tokens that are just words with slash but no file extension (e.g., Grant/Revoke)
        if t.contains('/') && t.len() < 120 {
            let parts: Vec<&str> = t.split('/').collect();
            if parts.len() >= 2 && parts.iter().all(|p| !p.is_empty() && p.len() < 40) {
                let last = parts.last().unwrap();
                // Require last part to have a valid file extension to be considered a file path
                if last.contains('.') {
                    let ext = last.rsplit('.').next().unwrap_or("").to_lowercase();
                    if ["rs","ts","tsx","js","jsx","py","go","java","json","toml","md","html","css","yaml","yml","sh","lock","txt","toml","rs","ts"].contains(&ext.as_str()) {
                        add(t);
                    }
                }
            }
        } else if t.contains('.') && !t.contains('/') {
            // Single filename like "main.rs" or "AuthService.ts" — only count if it looks like a file
            // Allow CamelCase + extension
            if t.len() < 60 && t.chars().filter(|c| *c == '.').count() == 1 {
                let ext = t.rsplit('.').next().unwrap_or("").to_lowercase();
                if ["rs","ts","tsx","js","jsx","py","go","java","json","toml","md","html","css","yaml","yml","sh","rs","ts"].contains(&ext.as_str()) {
                    // Reject if it looks like a sentence fragment (contains space already filtered) and not too short
                    let name_part = t.split('.').next().unwrap_or("");
                    if name_part.len() >= 2 && name_part.chars().any(|c| c.is_ascii_alphabetic()) {
                        add(t);
                    }
                }
            }
        }
    }

    // 3. Also look for markdown inline `src/...`
    for cap in text.split('`') {
        // odd splits are inside backticks? Actually splitting by ` gives alternating outside/inside
    }
    // For now, the token scan already caught those.

    out
}

fn extract_blocks<'a>(text: &'a str, marker: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut idx = 0;
    while let Some(pos) = text[idx..].find(marker) {
        let start = idx + pos;
        let end = text[start..].find('\n').map(|n| start + n).unwrap_or(text.len());
        out.push(&text[start..end.min(start+200)]);
        idx = end;
        if idx >= text.len() { break; }
    }
    out
}

/// Extract symbol mentions: `functionName()` , `ClassName` , etc.
/// Simple heuristic: words inside backticks, or CamelCase, or snake_case followed by ()
pub fn extract_symbols(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    // Backtick spans
    let mut in_tick = false;
    let mut tick_content = String::new();
    for c in text.chars() {
        if c == '`' {
            if in_tick {
                let t = tick_content.trim().to_string();
                if t.len() >= 2 && t.len() < 60 && !t.contains(' ') && t.chars().any(|c| c.is_alphanumeric()) {
                    if !seen.contains(&t) { seen.insert(t.clone()); out.push(t); }
                }
                tick_content.clear();
            }
            in_tick = !in_tick;
        } else if in_tick {
            tick_content.push(c);
        }
    }
    // Also find word() patterns
    for word in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        let w = word.trim();
        if w.len() >= 3 && w.len() < 40 && w.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
            // Could be symbol, but don't overflag common words
            if ["the","and","for","with","from","this","that","have","will","function","project","file","code"].contains(&w.to_lowercase().as_str()) {
                continue;
            }
            // Check if followed by () in original text? Simplified: if text contains "w(" near
            if text.contains(&format!("{}(", w)) || text.contains(&format!("{}.", w)) {
                if !seen.contains(w) { seen.insert(w.to_string()); out.push(w.to_string()); }
            }
        }
    }
    out.into_iter().take(20).collect()
}

pub fn verify_response(
    response: &str,
    project: &Project,
    files: &[ProjectFile],
    grounding_mode: &str,
) -> VerifierReport {
    let file_set: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let file_set_names: HashSet<String> = files.iter().map(|f| f.name.clone()).collect();
    let mentions = extract_file_mentions(response);
    let mut verifications = Vec::new();
    let mut invented = Vec::new();
    let mut verified = Vec::new();
    for m in mentions {
        // Check exact path or basename
        let exists = file_set.contains(&m) || file_set_names.contains(&m) || file_set.iter().any(|p| p.ends_with(&format!("/{}", m)) || p == &m);
        let kind = if exists { "file".to_string() } else { "invented".to_string() };
        if exists { verified.push(m.clone()); } else { invented.push(m.clone()); }
        verifications.push(FileVerification { claimed: m, exists, kind });
    }

    // Symbol checks via grep (lightweight: read files and search)
    let symbols = extract_symbols(response);
    let mut symbol_checks = Vec::new();
    if !symbols.is_empty() && !files.is_empty() {
        // Build content cache for current project (reuse list but read files)
        let mut content_cache: HashMap<String, String> = HashMap::new();
        for f in files.iter().filter(|f| !f.is_directory).take(100) {
            if let Ok(c) = core_fs::read_project_file(project, &f.path) {
                content_cache.insert(f.path.clone(), c);
            }
        }
        for sym in symbols {
            let mut found_files = Vec::new();
            for (path, content) in &content_cache {
                if content.contains(&sym) {
                    found_files.push(path.clone());
                    if found_files.len() >= 3 { break; }
                }
            }
            symbol_checks.push(SymbolCheck { symbol: sym, found: !found_files.is_empty(), files: found_files });
        }
    }

    // Edit block verification (SEARCH must exist)
    let mut edit_errors = Vec::new();
    // Look for <EDIT> blocks and check SEARCH exists
    let mut idx = 0;
    while let Some(pos) = response[idx..].find("<EDIT>") {
        let start = idx + pos;
        let end = response[start..].find("</EDIT>").map(|n| start + n + 7).unwrap_or(response.len());
        let block = &response[start..end.min(response.len())];
        // Extract FILE and SEARCH
        let file_path = extract_tag(block, "FILE:");
        let search = extract_tag(block, "SEARCH:");
        if let (Some(fp), Some(srch)) = (file_path, search) {
            let fp = fp.trim();
            let srch = srch.trim();
            if !srch.is_empty() && !fp.is_empty() {
                // Check file exists
                if !file_set.contains(fp) {
                    edit_errors.push(format!("EDIT file not in project: {}", fp));
                } else if let Ok(content) = core_fs::read_project_file(project, fp) {
                    if !content.contains(srch) {
                        edit_errors.push(format!("EDIT SEARCH not found in {} (check whitespace): {:.60}...", fp, srch));
                    }
                }
            }
        }
        idx = end;
        if idx >= response.len() { break; }
    }

    let total = verifications.len();
    let invented_count = invented.len();
    let score = if total == 0 { 0.0 } else { invented_count as f32 / total as f32 };
    let is_hallucinated = match grounding_mode {
        "strict" => invented_count > 0 || score > 0.0,
        "balanced" => score > 0.3 || invented_count > 2,
        _ => score > 0.6,
    };

    VerifierReport {
        mentioned_files: verifications,
        invented_files: invented.clone(),
        verified_files: verified,
        total_mentioned: total,
        invented_count,
        hallucination_score: score,
        is_hallucinated,
        grounding_mode: grounding_mode.to_string(),
        symbol_checks,
        edit_block_errors: edit_errors,
    }
}

fn extract_tag(block: &str, tag: &str) -> Option<String> {
    let pos = block.find(tag)?;
    let after = &block[pos + tag.len()..];
    // Find next tag or end
    let next_tags = ["SEARCH:", "REPLACE:", "FILE:", "CONTENT:", "</EDIT>", "</CREATE_FILE>"];
    let mut end = after.len();
    for nt in next_tags {
        if nt == tag { continue; }
        if let Some(p) = after.find(nt) {
            if p < end { end = p; }
        }
    }
    Some(after[..end].trim().to_string())
}

/// Quick check for deterministic inventory/project-name queries without LLM
pub fn is_inventory_query(text: &str) -> bool {
    let t = text.to_lowercase();
    // Mirrors frontend/App.tsx:1258
    (t.contains("inventory") && (t.contains("file") || t.contains("directories") || t.contains("folders") || t.contains("project") || t.contains("codebase")))
        || (t.contains("inspect") && (t.contains("file") || t.contains("directories") || t.contains("folders") || t.contains("project structure") || t.contains("codebase")))
        || ((t.contains("list") || t.contains("show")) && (t.contains("file") || t.contains("directories") || t.contains("folders")))
        || (t.contains("what") && (t.contains("file") || t.contains("directories") || t.contains("folders")) && (t.contains("project") || t.contains("codebase")))
        || t.trim() == "files" || t.trim() == "ls"
}

pub fn is_project_name_query(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("what is") && t.contains("project name")
        || t.contains("what's") && t.contains("project name")
        || t.contains("name of") && t.contains("project")
        || t.contains("project name") && (t.contains("what") || t.contains("which"))
}

pub fn deterministic_inventory_response(project: &Project, files: &[ProjectFile]) -> String {
    let real: String = files.iter().map(|f| if f.is_directory { format!("[DIR] {}", f.path) } else { format!("[FILE] {}", f.path) }).collect::<Vec<_>>().join("\n");
    format!("PROJECT INVENTORY\n\n{}\n\nTotal items: {} (verified via `files list`, no hallucination)", real, files.len())
}

/// Inject `[1]` citations after verified file mentions (ragground-style). Pure Rust fallback for --show-verifier citation view.
pub fn inject_citations(response: &str, report: &VerifierReport) -> String {
    let mut cited = response.to_string();
    for (idx, vf) in report.verified_files.iter().enumerate() {
        let tag = format!("{} [{}]", vf, idx+1);
        if cited.contains(vf) && !cited.contains(&tag) {
            cited = cited.replacen(vf, &tag, 1);
        }
    }
    cited
}

/// Find finetune/verify.py for external Python verifier (groundrails + LettuceDetect).
/// Checks: LOCAL_AI_VERIFY_SCRIPT env, ./finetune/verify.py, ../finetune/verify.py, <project>/finetune/verify.py
fn find_verify_script(project: &Project) -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("LOCAL_AI_VERIFY_SCRIPT") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() { return Some(pb); }
    }
    // Try candidate paths relative to cwd and project folder
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let candidates = vec![
        cwd.join("finetune/verify.py"),
        cwd.join("../finetune/verify.py"),
        cwd.join("../../finetune/verify.py"),
        // relative to project folder if attached
        project.folder_path.as_ref().map(|fp| std::path::PathBuf::from(fp).join("finetune/verify.py")).unwrap_or_default(),
        project.folder_path.as_ref().map(|fp| std::path::PathBuf::from(fp).join("../finetune/verify.py")).unwrap_or_default(),
        // binary dir fallback: try to locate workspace root via CARGO_MANIFEST_DIR at compile time
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../finetune/verify.py"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../finetune/verify.py"),
    ];
    for c in candidates {
        if !c.as_os_str().is_empty() && c.exists() {
            return Some(c);
        }
    }
    None
}

/// Try external Python verifier (groundrails/lettucedetect/ragground/groundlens) via subprocess.
/// Returns Some(report) if Python script succeeds, None if not available or fails (caller falls back to lexical).
/// Called when --show-verifier and script exists, or in strict mode if user enables external verify.
pub fn try_verify_with_python(
    response: &str,
    project: &Project,
    files: &[ProjectFile],
    grounding_mode: &str,
) -> Option<VerifierReport> {
    let script = find_verify_script(project)?;
    // Write answer to temp file to avoid arg length limits and escaping (cross-platform temp_dir)
    let mut tmp = std::env::temp_dir().join(format!("local-ai-answer-{}.txt", uuid::Uuid::new_v4()));
    if let Err(_) = std::fs::write(&tmp, response) {
        tmp = std::env::temp_dir().join("local-ai-answer.txt");
        if std::fs::write(&tmp, response).is_err() { return None; }
    }
    // Choose python binary — cross-platform: Windows uses `python`/`py`, Unix uses `python3`/`python`
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &["python", "py", "python3"]
    } else {
        &["python3", "python"]
    };
    let mut python: Option<&str> = None;
    for cand in candidates {
        if std::process::Command::new(*cand).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
            python = Some(*cand);
            break;
        }
    }
    let python = match python {
        Some(p) => p,
        None => {
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
    };
    // Build source args: use project file list as --sources (first 20 to avoid too many args)
    // For external, we pass --sources-dir as project root for full scan, plus explicit --sources for precision
    // Use absolute paths so Python can find files regardless of cwd
    let file_args: Vec<String> = files.iter().filter(|f| !f.is_directory).take(20).filter_map(|f| {
        // Validate path before joining — prevent absolute or traversal from poisoned project file list
        let p = std::path::Path::new(&f.path);
        if p.is_absolute() || p.components().any(|c| c == std::path::Component::ParentDir) {
            return None;
        }
        if let Some(root) = project.folder_path.as_deref() {
            Some(std::path::Path::new(root).join(p).to_string_lossy().to_string())
        } else { Some(f.path.clone()) }
    }).collect();
    let mut cmd = std::process::Command::new(python);
    cmd.arg(&script)
        .arg("--answer-file").arg(&tmp)
        .arg("--mode").arg(grounding_mode)
        .arg("--backend").arg("auto")
        .arg("--json");
    if let Some(root) = project.folder_path.as_deref() {
        cmd.arg("--sources-dir").arg(root);
    }
    if !file_args.is_empty() {
        cmd.arg("--sources");
        for fa in file_args { cmd.arg(fa); }
    }
    // Timeout 5s (like provider health_check) — avoid hanging on large model load
    let output = {
        use std::sync::mpsc;
        use std::time::Duration;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let out = cmd.output();
            let _ = tx.send(out);
        });
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(res) => res,
            Err(_) => {
                let _ = std::fs::remove_file(&tmp);
                return None;
            }
        }
    };
    let _ = std::fs::remove_file(&tmp);
    let out = match output {
        Ok(o) if o.status.success() || o.status.code() == Some(2) => o, // 2 = hallucinated in strict but still valid report
        Ok(o) => {
            // Python script printed error, ignore
            let _stderr = String::from_utf8_lossy(&o.stderr);
            return None;
        }
        Err(_) => return None,
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Parse JSON report
    let ext: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return None,
    };
    // Map external JSON to VerifierReport (compatible fields)
    // External uses same field names as Rust lexical, so we can try to deserialize
    // But external may have extra fields; we extract known ones
    let invented_files: Vec<String> = ext.get("invented_files").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    let verified_files: Vec<String> = ext.get("verified_files").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    let total_mentioned = ext.get("total_mentioned").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let invented_count = ext.get("invented_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let hallucination_score = ext.get("hallucination_score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let is_hallucinated = ext.get("is_hallucinated").and_then(|v| v.as_bool()).unwrap_or(invented_count>0);
    let grounding = ext.get("grounding_mode").and_then(|v| v.as_str()).unwrap_or(grounding_mode).to_string();
    let _backend = ext.get("backend").and_then(|v| v.as_str()).unwrap_or("python").to_string();

    // For symbol_checks and edit_block_errors, try to parse but fallback to lexical ones if missing
    let lexical = verify_response(response, project, files, grounding_mode);
    let symbol_checks = if let Some(arr) = ext.get("symbol_checks").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| {
            let symbol = v.get("symbol").and_then(|s| s.as_str())?.to_string();
            let found = v.get("found").and_then(|b| b.as_bool()).unwrap_or(false);
            let files = v.get("files").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            Some(SymbolCheck { symbol, found, files })
        }).collect()
    } else { lexical.symbol_checks.clone() };

    let edit_block_errors = if let Some(arr) = ext.get("edit_block_errors").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
    } else { lexical.edit_block_errors.clone() };

    let mentioned_files = if let Some(arr) = ext.get("mentioned_files").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| {
            let claimed = v.get("claimed").and_then(|s| s.as_str())?.to_string();
            let exists = v.get("exists").and_then(|b| b.as_bool()).unwrap_or(false);
            let kind = v.get("kind").and_then(|s| s.as_str()).unwrap_or(if exists {"file"} else {"invented"}).to_string();
            Some(FileVerification { claimed, exists, kind })
        }).collect()
    } else { lexical.mentioned_files.clone() };

    // If external backend is more permissive, we keep lexical invented_files as source of truth for strict mode
    // but annotate backend
    Some(VerifierReport {
        mentioned_files,
        invented_files,
        verified_files,
        total_mentioned,
        invented_count,
        hallucination_score,
        is_hallucinated,
        grounding_mode: grounding,
        symbol_checks,
        edit_block_errors,
    })
}

/// Hybrid verify: lexical + optional Python external (groundrails/LettuceDetect). Merges results, lexical is fallback.
pub fn verify_response_hybrid(
    response: &str,
    project: &Project,
    files: &[ProjectFile],
    grounding_mode: &str,
    try_external: bool,
) -> VerifierReport {
    let lexical = verify_response(response, project, files, grounding_mode);
    if !try_external {
        return lexical;
    }
    if let Some(ext) = try_verify_with_python(response, project, files, grounding_mode) {
        // Prefer external's hallucination_score but keep lexical invented_files if external is more lenient and strict mode
        // For strict, any invented from either is hallucinated
        if grounding_mode == "strict" && lexical.is_hallucinated && !ext.is_hallucinated {
            // Keep lexical's hallucination flag, but merge verified/invented from union
            let mut merged_invented = ext.invented_files.clone();
            for inv in lexical.invented_files.iter() {
                if !merged_invented.contains(inv) { merged_invented.push(inv.clone()); }
            }
            return VerifierReport {
                is_hallucinated: true,
                invented_files: merged_invented.clone(),
                invented_count: merged_invented.len(),
                hallucination_score: lexical.hallucination_score.max(ext.hallucination_score),
                ..ext
            };
        }
        return ext;
    }
    lexical
}
