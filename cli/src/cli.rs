use clap::{Parser, Subcommand};
use crate::commands::{project, models, chat, files, exec, analyze, finetune, doctor, config, index};

#[derive(Parser)]
#[command(name = "local-ai", version, about = "Local AI CLI — local model workspace + finetuning (all OS)")]
pub struct Cli {
    /// Provider: auto | lmstudio | ollama | llamacpp | generic (auto = autodetect Ollama → LM Studio → llama.cpp)
    #[arg(long, global = true)]
    pub provider: Option<String>,

    /// Provider base URL (overrides config). Examples: http://localhost:1234/v1 (LM Studio) or http://localhost:11434 (Ollama)
    #[arg(long, global = true, env = "LOCAL_AI_URL")]
    pub url: Option<String>,

    /// Deprecated: use --url. Kept for backwards compat with LM Studio
    #[arg(long, global = true, env = "LOCAL_AI_LM_STUDIO_URL", hide = true)]
    pub lm_studio_url: Option<String>,

    /// Grounding mode: strict | balanced | creative (strict = lowest hallucination)
    #[arg(long, global = true)]
    pub grounding: Option<String>,

    /// App mode: plan (read-only, no writes/exec) | build (allow writes) — plan can't build anything
    #[arg(long, global = true, env = "LOCAL_AI_MODE")]
    pub mode: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage projects (attach local folders, persistent history)
    Project(project::ProjectArgs),
    /// List / check local models (LM Studio + Ollama + any provider)
    Models(models::ModelsArgs),
    /// Chat with local model (streaming, project-aware, grounded)
    Chat(chat::ChatArgs),
    /// Project file operations (list, read, write, delete)
    Files(files::FilesArgs),
    /// Execute a shell command inside a project
    Exec(exec::ExecArgs),
    /// Analyze project with AI (architecture, explain, debug)
    Analyze(analyze::AnalyzeArgs),
    /// Fine-tune local models (QLoRA, export to GGUF/LM Studio)
    Finetune(finetune::FinetuneArgs),
    /// Check all providers health + embeddings
    Doctor(doctor::DoctorArgs),
    /// Manage config (~/.config/local-ai/config.toml)
    Config(config::ConfigArgs),
    /// Index project for hybrid retrieval (embeddings)
    Index(index::IndexArgs),
}
