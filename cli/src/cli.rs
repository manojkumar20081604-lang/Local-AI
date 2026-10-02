use clap::{Parser, Subcommand};
use crate::commands::{project, models, chat, files, exec, analyze, finetune, doctor, config, graph, index, git, memory, mission, browse, plan, debug, agent, bench, dataset, metrics, propose, tui};

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

    /// Extra MCP tool servers (repeatable): --mcp filesystem --mcp github
    #[arg(long, global = true)]
    pub mcp: Vec<String>,

    /// Plain output: no colors or animations (scripts/CI friendly)
    #[arg(long, global = true)]
    pub plain: bool,

    /// Free-form goal with no subcommand: `local-ai "fix the PDF crash"`
    /// routes to the agent loop in the current directory.
    pub prompt: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
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
    /// Git intelligence (read-only: status, diff, log, blame)
    Git(git::GitArgs),
    /// Build an approvable execution graph for a goal (read-only)
    Plan(plan::PlanArgs),
    /// Execute a plan graph step by step (preview with --dry-run)
    #[command(name = "exec-plan")]
    ExecPlan(plan::ExecPlanArgs),
    /// Self-debugging loop: run tests, parse failures, fix, re-test
    Debug(debug::DebugArgs),
    /// Code graph: symbols + imports (read-only)
    Graph(graph::GraphArgs),
    /// Three-tier memory: project|user|task (show is read-only)
    Memory(memory::MemoryArgs),
    /// Orchestrated specialists: planner → researcher → coder → tester → reviewer
    Agent(agent::AgentArgs),
    /// Persisted missions: create → resume → show (command-center feel)
    Mission(mission::MissionArgs),
    /// Browse the web: fetch pages, extract code blocks, cite URLs
    Browse(browse::BrowseArgs),
    /// Benchmark studio: TPS, TTFT, pass@1 on fixture tasks
    Bench(bench::BenchArgs),
    /// Dataset feedback loop: approved interactions → QLoRA JSONL
    Dataset(dataset::DatasetArgs),
    /// Project health: retrieval hit-rate, verifier reject-rate, test pass-rate
    Metrics(metrics::MetricsArgs),
    /// Self-improvement proposals as inspectable plan-graphs
    Propose(propose::ProposeArgs),
    /// Fullscreen anime command center (needs an interactive terminal)
    Tui(tui::TuiArgs),
}
