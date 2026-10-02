//! `local-ai tui` — fullscreen anime command center (presentation layer).
//!
//! ```bash
//! local-ai tui --project MyApp
//! local-ai tui --no-animation --ascii --theme matrix
//! ```
//!
//! The engine is untouched: the TUI subscribes to the agent event bus and
//! routes approvals back through oneshot responders. Refuses non-terminals
//! (use the plain subcommands in scripts/CI).

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
pub struct TuiArgs {
    /// Project id/name/path (default: current dir)
    #[arg(long)]
    pub project: Option<String>,

    /// Skip the startup animation
    #[arg(long)]
    pub no_animation: bool,

    /// ASCII faces (also auto-enabled without a UTF-8 locale)
    #[arg(long)]
    pub ascii: bool,

    /// Theme: midnight|cyberpunk|anime|matrix|monochrome|minimal|custom
    #[arg(long)]
    pub theme: Option<String>,
}

pub async fn handle(args: TuiArgs) -> Result<()> {
    crate::tui::app::run(crate::tui::app::TuiOptions {
        project: args.project,
        no_animation: args.no_animation,
        ascii: args.ascii,
        theme: args.theme,
    })
    .await
}
