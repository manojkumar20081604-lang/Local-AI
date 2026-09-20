use anyhow::Result;
use clap::Parser;
use console::style;

use crate::core::{fs as core_fs, projects};

#[derive(Parser)]
pub struct ExecArgs {
    /// Shell command to run (e.g. "npm test" or "cargo build")
    #[arg(required = true, trailing_var_arg = true)]
    pub command: Vec<String>,

    /// Project id/name/path (default: current dir)
    #[arg(long)]
    pub project: Option<String>,

    /// Show exit code only
    #[arg(long)]
    pub quiet: bool,
}

pub async fn handle(args: ExecArgs) -> Result<()> {
    crate::core::config::require_build_mode("exec")?;
    let proj = projects::resolve_project(args.project)?;
    let cmd = args.command.join(" ");
    if !args.quiet {
        println!("{} {} {}", style("$").dim(), style(&cmd).cyan(), style(format!("(in {})", proj.folder_path.as_deref().unwrap_or("."))).dim());
    }
    let result = core_fs::run_project_command(&proj, &cmd)?;
    if !result.stdout.is_empty() {
        print!("{}", result.stdout);
        if !result.stdout.ends_with('\n') { println!(); }
    }
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
        if !result.stderr.ends_with('\n') { eprintln!(); }
    }
    if !args.quiet {
        if result.success {
            eprintln!("{} exit {}", style("✓").green(), result.exit_code.unwrap_or(0));
        } else {
            eprintln!("{} exit {}", style("✗").red(), result.exit_code.unwrap_or(1));
        }
    }
    if !result.success {
        std::process::exit(result.exit_code.unwrap_or(1));
    }
    Ok(())
}
