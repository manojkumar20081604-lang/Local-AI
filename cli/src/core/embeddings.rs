use anyhow::{Context, Result};
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};

/// Trait for local embeddings — used for hybrid retrieval.
/// Implementations: FastEmbed (ONNX, offline), Ollama (via /api/embeddings), TfIdf fallback.
pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn name(&self) -> &str;
    fn dim(&self) -> usize;
}

// ---------- Utility ----------

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() { return 0.0; }
    let (mut dot, mut na, mut nb) = (0.0, 0.0, 0.0);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na.sqrt() * nb.sqrt()) }
}

pub fn normalize(v: &mut [f32]) {
    let n: f32 = v.iter().map(|x| x*x).sum::<f32>().sqrt();
    if n > 0.0 { for x in v.iter_mut() { *x /= n; } }
}

// ---------- FastEmbed (default, offline, pure Rust ONNX) ----------

pub struct FastEmbedEmbedder {
    model: Arc<Mutex<fastembed::TextEmbedding>>,
    dim: usize,
}

impl FastEmbedEmbedder {
    pub fn try_new() -> Result<Self> {
        Self::try_new_with_model("BAAI/bge-small-en-v1.5")
    }

    pub fn try_new_with_model(model_name: &str) -> Result<Self> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        // Map string to enum; fallback to BGE Small V1.5. See fastembed 5.17 text_embedding.rs
        let model = match model_name {
            "BAAI/bge-small-en-v1.5" | "bge-small" | "BAAI/bge-small-en" => EmbeddingModel::BGESmallENV15,
            "BAAI/bge-base-en-v1.5" | "bge-base" => EmbeddingModel::BGESmallENV15, // Base not in 5.17, fallback to small
            "BAAI/bge-large-en-v1.5" | "bge-large" => EmbeddingModel::BGELargeENV15,
            "sentence-transformers/all-MiniLM-L6-v2" | "all-minilm-l6-v2" => EmbeddingModel::AllMiniLML6V2,
            "nomic-ai/nomic-embed-text-v1.5" | "nomic" => EmbeddingModel::NomicEmbedTextV15,
            _ => EmbeddingModel::BGESmallENV15,
        };
        let embedding = TextEmbedding::try_new(
            InitOptions::new(model.clone()).with_show_download_progress(true)
        ).context("Failed to init FastEmbed")?;
        let dim = match model {
            EmbeddingModel::BGESmallENV15 => 384,
            EmbeddingModel::BGESmallENV15Q => 384,
            EmbeddingModel::BGELargeENV15 => 1024,
            EmbeddingModel::BGELargeENV15Q => 1024,
            EmbeddingModel::AllMiniLML6V2 => 384,
            EmbeddingModel::NomicEmbedTextV15 => 768,
            EmbeddingModel::NomicEmbedTextV15Q => 768,
            _ => 384,
        };
        Ok(Self { model: Arc::new(Mutex::new(embedding)), dim })
    }
}

impl Embedder for FastEmbedEmbedder {
    fn name(&self) -> &str { "fastembed" }
    fn dim(&self) -> usize { self.dim }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut guard = self.model.lock().map_err(|_| anyhow::anyhow!("FastEmbed lock poisoned"))?;
        let embeddings = guard.embed(texts.to_vec(), None).context("FastEmbed embed failed")?;
        Ok(embeddings)
    }
}

// ---------- Ollama Embed (if Ollama is up) ----------

pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    dim: usize,
}

impl OllamaEmbedder {
    pub fn new(base_url: String, model: String) -> Self {
        // dim guessed; Ollama will return actual dim per model. nomic-embed-text 768, all-minilm 384, bge-small 384
        let dim = match model.as_str() {
            "nomic-embed-text" | "nomic-embed-text:latest" => 768,
            "all-minilm" | "all-minilm:latest" => 384,
            "bge-small" => 384,
            "bge-large" => 1024,
            _ => 768,
        };
        Self { base_url: base_url.trim_end_matches('/').to_string(), model, dim }
    }

    pub fn health_check(&self) -> bool {
        // Blocking check for simplicity; called from async context via spawn_blocking in future if needed
        // Here we do sync via reqwest blocking is not available, so use quick reqwest async via futures::executor::block_on? Instead expose async.
        false
    }

    pub async fn health_check_async(&self) -> bool {
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(2)).build();
        let Ok(c) = client else { return false };
        // Ollama embeddings endpoint is POST /api/embed or /api/embeddings
        c.get(format!("{}/api/tags", self.base_url)).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }

    pub async fn embed_async(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            // Ollama /api/embeddings: {model, prompt}
            // /api/embed: {model, input}  supports array
            // Try /api/embed first (newer)
            let body = serde_json::json!({"model": self.model, "input": text});
            let resp = client.post(format!("{}/api/embed", self.base_url)).json(&body).send().await;
            let embeddings: Vec<Vec<f32>> = if let Ok(r) = resp {
                if r.status().is_success() {
                    #[derive(serde::Deserialize)] struct Resp { embeddings: Vec<Vec<f32>> }
                    if let Ok(j) = r.json::<Resp>().await {
                        j.embeddings
                    } else {
                        // Try older /api/embeddings
                        let body2 = serde_json::json!({"model": self.model, "prompt": text});
                        let r2 = client.post(format!("{}/api/embeddings", self.base_url)).json(&body2).send().await.context("Ollama embeddings failed")?;
                        #[derive(serde::Deserialize)] struct Resp2 { embedding: Vec<f32> }
                        let j2: Resp2 = r2.json().await.context("Parse Ollama embeddings")?;
                        vec![j2.embedding]
                    }
                } else {
                    // fallback to /api/embeddings
                    let body2 = serde_json::json!({"model": self.model, "prompt": text});
                    let r2 = client.post(format!("{}/api/embeddings", self.base_url)).json(&body2).send().await.context("Ollama embeddings failed")?;
                    #[derive(serde::Deserialize)] struct Resp2 { embedding: Vec<f32> }
                    let j2: Resp2 = r2.json().await.context("Parse Ollama embeddings")?;
                    vec![j2.embedding]
                }
            } else {
                let body2 = serde_json::json!({"model": self.model, "prompt": text});
                let r2 = client.post(format!("{}/api/embeddings", self.base_url)).json(&body2).send().await.context("Ollama embeddings failed")?;
                #[derive(serde::Deserialize)] struct Resp2 { embedding: Vec<f32> }
                let j2: Resp2 = r2.json().await.context("Parse Ollama embeddings")?;
                vec![j2.embedding]
            };
            out.extend(embeddings);
        }
        Ok(out)
    }
}

impl Embedder for OllamaEmbedder {
    fn name(&self) -> &str { "ollama" }
    fn dim(&self) -> usize { self.dim }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Sync wrapper for trait — block_on async
        let rt = tokio::runtime::Handle::try_current();
        if let Ok(handle) = rt {
            // If we are inside tokio, we cannot block_on directly (would panic). Use block_in_place? Instead spawn and block.
            // For simplicity, use futures::executor::block_on via handle.block_on if not already in runtime? But we are in runtime, so we need to use tokio::task::block_in_place? Safer: just run async via handle.
            // Use handle.block_on only if we are not in async context? This is messy. Instead require async embed for Ollama.
            // Fallback: use reqwest::blocking? Not available. So just error and let caller use async version.
            anyhow::bail!("OllamaEmbedder::embed sync called inside tokio runtime — use embed_async")
        } else {
            // Outside runtime, create one
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(self.embed_async(texts))
        }
    }
}

// ---------- TfIdf Fallback (no model, deterministic, CPU) ----------
// Simple char-bigram hashing trick to produce dense vector (384 dim) for hybrid fallback without model download.
// Not as good as BGE, but avoids panic when offline and FastEmbed not yet downloaded.

pub struct TfIdfEmbedder {
    dim: usize,
}

impl TfIdfEmbedder {
    pub fn new(dim: usize) -> Self { Self { dim } }
    fn hash_embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        let lower = text.to_lowercase();
        // char bigrams + token hash
        let chars: Vec<char> = lower.chars().collect();
        for w in lower.split_whitespace() {
            let h = blake3::hash(w.as_bytes());
            let bytes = h.as_bytes();
            for i in 0..8 {
                let idx = (bytes[i] as usize * 256 + bytes[i+1] as usize) % self.dim;
                v[idx] += 1.0;
            }
        }
        for i in 0..chars.len().saturating_sub(1) {
            let bigram = format!("{}{}", chars[i], chars[i+1]);
            let h = blake3::hash(bigram.as_bytes());
            let idx = (h.as_bytes()[0] as usize) % self.dim;
            v[idx] += 0.5;
        }
        // normalize
        let n: f32 = v.iter().map(|x| x*x).sum::<f32>().sqrt();
        if n > 0.0 { for x in &mut v { *x /= n; } }
        v
    }
}

impl Embedder for TfIdfEmbedder {
    fn name(&self) -> &str { "tfidf" }
    fn dim(&self) -> usize { self.dim }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.hash_embed(t)).collect())
    }
}

// ---------- Global factory ----------

static FASTEMBED_SINGLETON: OnceLock<Result<Arc<dyn Embedder>, String>> = OnceLock::new();

fn is_fastembed_cached(model_name: &str) -> bool {
    // Check hf-hub cache for model, e.g. ~/.cache/huggingface/hub/models--BAAI--bge-small-en-v1.5
    let cache_base = dirs::cache_dir().map(|p| p.join("huggingface/hub")).unwrap_or_else(|| std::env::temp_dir().join("huggingface/hub"));
    let safename = model_name.replace('/', "--");
    let model_dir = cache_base.join(format!("models--{}", safename));
    model_dir.exists()
        && fs::read_dir(&model_dir).map(|mut d| d.next().is_some()).unwrap_or(false)
        || {
            // Also check fastembed specific cache location via hf-hub default (may be different)
            // FastEmbed also caches ONNX files under hf-hub cache; if not found, treat as not cached
            // To avoid long download on slow network, we require cache to exist
            false
        }
}

pub fn get_embedder(cfg: &crate::core::config::AppConfig) -> Arc<dyn Embedder> {
    let provider = cfg.embeddings.provider.as_str();
    match provider {
        "ollama" => {
            let base = cfg.providers.ollama.url.clone();
            let model = cfg.embeddings.model.clone();
            Arc::new(OllamaEmbedder::new(base, if model.contains('/') { "nomic-embed-text".into() } else { model.clone() }))
        }
        "tfidf" => Arc::new(TfIdfEmbedder::new(384)),
        "fastembed" | _ => {
            // Only try FastEmbed if model is already cached, otherwise fallback to TfIdf to avoid 120MB download hang
            let model_name = &cfg.embeddings.model;
            if !is_fastembed_cached(model_name) {
                eprintln!("FastEmbed model {} not cached (would require download), using TfIdf fallback. Run `local-ai config set embeddings.provider fastembed` and ensure network if you want fastembed, or `local-ai index rebuild` will use TfIdf.", model_name);
                return Arc::new(TfIdfEmbedder::new(384));
            }
            let res = FASTEMBED_SINGLETON.get_or_init(|| {
                match FastEmbedEmbedder::try_new_with_model(model_name) {
                    Ok(e) => Ok(Arc::new(e) as Arc<dyn Embedder>),
                    Err(err) => Err(err.to_string()),
                }
            });
            match res {
                Ok(embedder) => embedder.clone(),
                Err(err) => {
                    eprintln!("FastEmbed init failed ({}), falling back to TfIdf", err);
                    Arc::new(TfIdfEmbedder::new(384))
                }
            }
        }
    }
}

/// Async variant that tries Ollama health check first if provider=auto or embeddings.provider=ollama
pub async fn get_embedder_async(cfg: &crate::core::config::AppConfig) -> Arc<dyn Embedder> {
    if cfg.embeddings.provider == "ollama" || cfg.embeddings.provider == "auto" {
        let ollama = OllamaEmbedder::new(cfg.providers.ollama.url.clone(), "nomic-embed-text".into());
        if ollama.health_check_async().await {
            if let Ok(models) = ollama_helper::list_ollama_models(&cfg.providers.ollama.url).await {
                // If nomic-embed-text present, use it
                if models.iter().any(|m| m.contains("nomic-embed-text") || m.contains("all-minilm") || m.contains("bge")) {
                    let best = models.iter().find(|m| m.contains("nomic-embed-text")).cloned().unwrap_or_else(|| "nomic-embed-text".into());
                    return Arc::new(OllamaEmbedder::new(cfg.providers.ollama.url.clone(), best));
                }
            }
            // Still return Ollama embedder even if model not listed — it will error and caller fallback
            return Arc::new(ollama);
        }
    }
    // Fallback to sync get
    get_embedder(cfg)
}

// Helper for Ollama list used above — expose simple function
pub mod ollama_helper {
    use anyhow::Result;
    pub async fn list_ollama_models(base_url: &str) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() { return Ok(vec![]); }
        #[derive(serde::Deserialize)] struct Tags { models: Option<Vec<Tag>> }
        #[derive(serde::Deserialize)] struct Tag { name: String }
        let json: Tags = resp.json().await?;
        Ok(json.models.unwrap_or_default().into_iter().map(|m| m.name).collect())
    }
}
