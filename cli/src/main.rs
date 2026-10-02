use clap::Parser;
use local_ai::cli::{Cli, Commands};
use local_ai::commands;
use local_ai::core::config::{ProviderKind, load_config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Resolve provider from flag -> env -> config file. Handle --provider parsing.
    let cli_provider_kind: Option<ProviderKind> = if let Some(ref s) = cli.provider {
        match s.parse::<ProviderKind>() {
            Ok(k) => Some(k),
            Err(e) => {
                eprintln!("Invalid --provider: {}", e);
                std::process::exit(1);
            }
        }
    } else { None };

    // Global grounding override if provided
    if let Some(g) = cli.grounding.clone() {
        std::env::set_var("LOCAL_AI_GROUNDING", g);
    }

    // Mode: plan (read-only) vs build (allow writes) — plan can't build anything
    if let Some(m) = cli.mode.clone() {
        let lower = m.to_lowercase();
        if lower != "plan" && lower != "build" {
            eprintln!("Invalid --mode: '{}', expected plan|build", m);
            std::process::exit(1);
        }
        std::env::set_var("LOCAL_AI_MODE", lower);
    }

    // Legacy + new URL handling via config resolver — set env for backwards compat layers
    // We don't set env globally here; provider dispatch will resolve it per-command.
    // But keep old behaviour for code that calls lm_studio_base_url()
    if let Some(url) = cli.url.clone() {
        std::env::set_var("LOCAL_AI_URL", url);
    }
    if let Some(url) = cli.lm_studio_url.clone() {
        std::env::set_var("LOCAL_AI_LM_STUDIO_URL", url.clone());
        // Also set generic URL if --url not set
        if cli.url.is_none() {
            std::env::set_var("LOCAL_AI_URL", url);
        }
    }

    // Store resolved provider in env for callee that needs it without passing explicitly
    // (chat/models will re-resolve from flags+config, but this helps simple calls)
    if let Some(k) = &cli_provider_kind {
        std::env::set_var("LOCAL_AI_PROVIDER", k.to_string());
    }

    match (cli.command, cli.prompt) {
        (Some(cmd), _) => match cmd {
        Commands::Project(cmd) => commands::project::handle(cmd).await?,
        Commands::Models(cmd) => commands::models::handle(cmd, cli_provider_kind, cli.url.clone(), cli.lm_studio_url.clone()).await?,
        Commands::Chat(cmd) => commands::chat::handle(cmd, cli_provider_kind, cli.url.clone(), cli.lm_studio_url.clone(), cli.mcp.clone()).await?,
        Commands::Files(cmd) => commands::files::handle(cmd).await?,
        Commands::Exec(cmd) => commands::exec::handle(cmd).await?,
        Commands::Analyze(cmd) => commands::analyze::handle(cmd, cli_provider_kind, cli.url.clone(), cli.lm_studio_url.clone()).await?,
        Commands::Finetune(cmd) => commands::finetune::handle(cmd).await?,
        Commands::Doctor(cmd) => commands::doctor::handle(cmd).await?,
        Commands::Config(cmd) => commands::config::handle(cmd).await?,
        Commands::Index(cmd) => commands::index::handle(cmd).await?,
        Commands::Graph(cmd) => commands::graph::handle(cmd).await?,
        Commands::Memory(cmd) => commands::memory::handle(cmd).await?,
        Commands::Git(cmd) => commands::git::handle(cmd).await?,
        Commands::Plan(cmd) => commands::plan::handle_plan(cmd).await?,
        Commands::ExecPlan(cmd) => commands::plan::handle_exec_plan(cmd).await?,
        Commands::Debug(cmd) => commands::debug::handle(cmd, cli_provider_kind, cli.url.clone(), cli.lm_studio_url.clone()).await?,
        Commands::Agent(cmd) => commands::agent::handle(cmd, cli_provider_kind, cli.url.clone(), cli.lm_studio_url.clone()).await?,
        Commands::Mission(cmd) => commands::mission::handle(cmd, cli_provider_kind, cli.url.clone(), cli.lm_studio_url.clone()).await?,
        Commands::Browse(cmd) => commands::browse::handle(cmd).await?,
        Commands::Bench(cmd) => commands::bench::handle(cmd, cli_provider_kind, cli.url.clone(), cli.lm_studio_url.clone()).await?,
        Commands::Dataset(cmd) => commands::dataset::handle(cmd).await?,
        Commands::Metrics(cmd) => commands::metrics::handle(cmd).await?,
        Commands::Propose(cmd) => commands::propose::handle(cmd).await?,
        },
        // Bare `local-ai "fix the crash"` → agent loop in the current directory.
        // Same foreground, bounded, approval-gated engine as `agent run`.
        (None, Some(prompt)) => {
            eprintln!("No command given — running agent for: {}", prompt);
            commands::agent::handle(
                commands::agent::AgentArgs {
                    command: commands::agent::AgentCommands::Run {
                        goal: prompt,
                        project: None,
                        max_steps: 20,
                        max_tool_calls: 50,
                        max_wall_secs: 600,
                        test_cmd: None,
                        model: None,
                        provider: None,
                        dry_run: false,
                        yes: false,
                        approve: Vec::new(),
                    },
                },
                cli_provider_kind,
                cli.url.clone(),
                cli.lm_studio_url.clone(),
            )
            .await?
        }
        // Bare `local-ai` → interactive chat REPL in the current directory.
        (None, None) => {
            eprintln!("No command given — opening chat in current directory (type /exit to quit)");
            commands::chat::handle(
                commands::chat::ChatArgs {
                    message: String::new(),
                    model: None,
                    project: None,
                    no_context: false,
                    no_save: false,
                    system: None,
                    show_context: false,
                    grounding: None,
                    provider: None,
                    url: None,
                    show_verifier: false,
                    no_verify: false,
                    verify_only: false,
                    tools: false,
                    route: false,
                },
                cli_provider_kind,
                cli.url.clone(),
                cli.lm_studio_url.clone(),
                cli.mcp.clone(),
            )
            .await?
        }
    }

    // Avoid unused warning for load_config import
    let _ = load_config as fn() -> anyhow::Result<local_ai::core::config::AppConfig>;

    Ok(())
}
