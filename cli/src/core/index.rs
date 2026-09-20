use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use super::embeddings::{Embedder, cosine};
use super::fs as core_fs;
use super::projects::Project;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Chunk {
    pub text: String,
    pub embedding: Vec<f32>,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub mtime: u64,
    pub hash: String,
    pub size: usize,
    pub file_embedding: Vec<f32>,
    pub chunks: Vec<Chunk>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectIndex {
    pub version: u32,
    pub project_id: String,
    pub created_at: String,
    pub files: Vec<FileEntry>,
    pub embedder_name: String,
    pub embedder_dim: usize,
}

fn cache_dir(project: &Project) -> Result<PathBuf> {
    let base = dirs::cache_dir().context("No cache dir")?;
    let dir = base.join("local-ai").join(&project.id);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn index_path(project: &Project) -> Result<PathBuf> {
    Ok(cache_dir(project)?.join("index.json"))
}

pub fn load_index(project: &Project) -> Result<Option<ProjectIndex>> {
    let path = index_path(project)?;
    if !path.exists() { return Ok(None); }
    let content = fs::read_to_string(&path)?;
    if content.trim().is_empty() { return Ok(None); }
    let idx: ProjectIndex = serde_json::from_str(&content).context("Failed to parse index.json")?;
    Ok(Some(idx))
}

pub fn save_index(project: &Project, idx: &ProjectIndex) -> Result<()> {
    let path = index_path(project)?;
    let json = serde_json::to_string_pretty(idx)?;
    fs::write(&path, json)?;
    Ok(())
}

fn file_mtime(path: &std::path::Path) -> u64 {
    fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn chunk_text(content: &str, chunk_size: usize, overlap: usize) -> Vec<(String, usize, usize)> {
    // Simple char-based chunking with overlap, preserves line boundaries when possible
    if content.len() <= chunk_size {
        return vec![(content.to_string(), 0, content.len())];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    while start < len {
        let end = (start + chunk_size).min(len);
        // Try to snap to newline boundary within last 200 chars
        let mut snap_end = end;
        if end < len {
            for i in (end.saturating_sub(200)..end).rev() {
                if chars[i] == '\n' { snap_end = i + 1; break; }
            }
        }
        let text: String = chars[start..snap_end].iter().collect();
        chunks.push((text, start, snap_end));
        if snap_end >= len { break; }
        start = snap_end.saturating_sub(overlap);
    }
    chunks
}

fn hash_content(content: &str) -> String {
    let hash = blake3::hash(content.as_bytes());
    hash.to_hex()[..16].to_string()
}

pub fn build_index(project: &Project, embedder: Arc<dyn Embedder>) -> Result<ProjectIndex> {
    let folder = project.folder_path.as_ref().context("Project has no folder")?;
    let root = PathBuf::from(folder);

    let files = core_fs::list_project_files(project)?;
    let mut entries = Vec::new();

    // Collect file contents first
    let mut file_contents: Vec<(String, String, String, u64)> = Vec::new(); // path, name, content, mtime
    for f in files.iter().filter(|f| !f.is_directory) {
        let abs = root.join(&f.path);
        let mtime = file_mtime(&abs);
        let content = match core_fs::read_project_file(project, &f.path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if content.trim().is_empty() { continue; }
        if content.len() > 500_000 { continue; } // skip huge files
        // Skip binaries already filtered by list but double-check
        if content.chars().any(|c| c == '\0') { continue; }
        file_contents.push((f.path.clone(), f.name.clone(), content, mtime));
    }

    if file_contents.is_empty() {
        anyhow::bail!("No indexable files found");
    }

    println!("Indexing {} files with {} ({}dim)...", file_contents.len(), embedder.name(), embedder.dim());

    // Batch embed file-level texts (first 2000 chars) + chunks
    // For efficiency, embed in batches of 8
    let mut all_texts: Vec<String> = Vec::new();
    let mut text_to_entry: Vec<(usize, usize)> = Vec::new(); // (file_idx, chunk_idx_or_file)
    // We will produce embeddings for each file's chunks + file-level
    // But for simplicity, first produce file-level embeddings via first 1500 chars
    for (fi, (_, _, content, _)) in file_contents.iter().enumerate() {
        let snippet = if content.len() > 1500 { content[..1500].to_string() } else { content.clone() };
        all_texts.push(snippet);
        text_to_entry.push((fi, usize::MAX)); // file-level sentinel
        for (chunk_text, _, _) in chunk_text(content, 1500, 200) {
            all_texts.push(chunk_text);
            text_to_entry.push((fi, 0)); // placeholder, we'll expand later
        }
    }

    // Actually we need accurate mapping: per file, chunks count variable, so we need per-file chunk texts
    // Rebuild all_texts correctly with per-file chunks
    all_texts.clear();
    text_to_entry.clear();
    let mut file_chunks: Vec<Vec<(String, usize, usize)>> = Vec::new();
    for (fi, (_, _, content, _)) in file_contents.iter().enumerate() {
        let chunks = chunk_text(content, 1500, 200);
        file_chunks.push(chunks.clone());
        // file-level
        let snippet = if content.len() > 1500 { content[..1500].to_string() } else { content.clone() };
        all_texts.push(snippet);
        text_to_entry.push((fi, usize::MAX));
        for (ct, _, _) in chunks {
            all_texts.push(ct);
            text_to_entry.push((fi, 0));
        }
    }

    // Batch embed 16 at a time
    let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(all_texts.len());
    for batch in all_texts.chunks(16) {
        let batch_strs: Vec<String> = batch.iter().cloned().collect();
        let embeds = embedder.embed(&batch_strs).context("Embed batch failed")?;
        all_embeddings.extend(embeds);
    }

    if all_embeddings.len() != all_texts.len() {
        anyhow::bail!("Embedding count mismatch: {} vs {}", all_embeddings.len(), all_texts.len());
    }

    // Reconstruct entries
    let mut embed_idx = 0;
    for (fi, (path, name, content, mtime)) in file_contents.into_iter().enumerate() {
        let hash = hash_content(&content);
        let file_emb = all_embeddings[embed_idx].clone();
        embed_idx += 1;
        let chunks_raw = file_chunks[fi].clone();
        let mut chunks = Vec::new();
        for (ct, s, e) in chunks_raw {
            let emb = all_embeddings[embed_idx].clone();
            embed_idx += 1;
            chunks.push(Chunk { text: ct, embedding: emb, start: s, end: e });
        }
        entries.push(FileEntry {
            path,
            name,
            mtime,
            hash,
            size: content.len(),
            file_embedding: file_emb,
            chunks,
        });
    }

    let idx = ProjectIndex {
        version: 2,
        project_id: project.id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        files: entries,
        embedder_name: embedder.name().to_string(),
        embedder_dim: embedder.dim(),
    };
    save_index(project, &idx)?;
    println!("✓ Indexed {} files → {}", idx.files.len(), index_path(project)?.display());
    Ok(idx)
}

pub fn needs_rebuild(project: &Project, idx: &ProjectIndex) -> bool {
    let folder = match &project.folder_path { Some(f) => PathBuf::from(f), None => return true };
    let files = match core_fs::list_project_files(project) { Ok(f) => f, Err(_) => return true };
    // Build filtered list as done in build_index (same filters)
    let mut filtered_paths = std::collections::HashSet::new();
    for f in files.iter().filter(|f| !f.is_directory) {
        let abs = folder.join(&f.path);
        // Quick check: try to read and apply same filters as build_index
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if content.trim().is_empty() { continue; }
        if content.len() > 500_000 { continue; }
        if content.chars().any(|c| c == '\0') { continue; }
        filtered_paths.insert(f.path.clone());
        // If this file is not in index or mtime changed, rebuild
        if let Some(entry) = idx.files.iter().find(|e| e.path == f.path) {
            let mtime = file_mtime(&abs);
            if entry.mtime != mtime { return true; }
            let hash = hash_content(&content);
            if entry.hash != hash { return true; }
        } else {
            return true; // new file that should be indexed
        }
    }
    // Check if any indexed file no longer exists or was filtered out differently
    if idx.files.len() != filtered_paths.len() { return true; }
    for entry in &idx.files {
        if !filtered_paths.contains(&entry.path) {
            return true; // file deleted or now filtered
        }
    }
    // Also check embedder dim mismatch already handled in ensure_index
    false
}

pub fn ensure_index(project: &Project, embedder: Arc<dyn Embedder>) -> Result<ProjectIndex> {
    if let Some(idx) = load_index(project)? {
        if !needs_rebuild(project, &idx) && idx.embedder_dim == embedder.dim() {
            return Ok(idx);
        }
        println!("Index stale ({} files, embedder {}), rebuilding…", idx.files.len(), idx.embedder_name);
    }
    build_index(project, embedder)
}

pub fn query_index(index: &ProjectIndex, query_emb: &[f32], top_k: usize) -> Vec<(FileEntry, f32, usize)> {
    // Score each file by max chunk cosine (or file_embedding)
    let mut scored: Vec<(FileEntry, f32, usize)> = Vec::new();
    for entry in &index.files {
        let mut best = cosine(query_emb, &entry.file_embedding);
        let mut best_chunk_idx = 0;
        for (i, chunk) in entry.chunks.iter().enumerate() {
            let s = cosine(query_emb, &chunk.embedding);
            if s > best {
                best = s;
                best_chunk_idx = i;
            }
        }
        scored.push((entry.clone(), best, best_chunk_idx));
    }
    scored.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(top_k).collect()
}

pub fn index_status(project: &Project) -> Result<String> {
    let path = index_path(project)?;
    if !path.exists() {
        return Ok(format!("No index at {} — run `local-ai index rebuild --project {}`", path.display(), project.id));
    }
    let idx = load_index(project)?.context("Failed to load index")?;
    let age = chrono::DateTime::parse_from_rfc3339(&idx.created_at).map(|d| chrono::Utc::now().signed_duration_since(d.with_timezone(&chrono::Utc)).num_minutes()).unwrap_or(0);
    let stale = needs_rebuild(project, &idx);
    Ok(format!(
        "Index: {} files, embedder={} ({}dim), created {} ({}m ago), stale={}, path={}",
        idx.files.len(), idx.embedder_name, idx.embedder_dim, idx.created_at, age, stale, path.display()
    ))
}
