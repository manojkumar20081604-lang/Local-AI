//! Bare invocation tests (P0): `local-ai` → REPL, `local-ai "goal"` → agent.
//!
//! Parsing only (no LLM, no execution): asserts clap routes bare mode,
//! free-form prompts, and explicit subcommands to the right slots.

use clap::Parser;
use local_ai::cli::{Cli, Commands};

fn parse(args: &[&str]) -> Cli {
    let mut full = vec!["local-ai"];
    full.extend(args);
    Cli::try_parse_from(full).expect("parse should succeed")
}

#[test]
fn test_bare_no_args_routes_to_repl() {
    let cli = parse(&[]);
    assert!(cli.command.is_none());
    assert!(cli.prompt.is_none());
}

#[test]
fn test_bare_prompt_routes_to_agent() {
    let cli = parse(&["fix the PDF crash"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.prompt.as_deref(), Some("fix the PDF crash"));
}

#[test]
fn test_explicit_subcommand_still_wins() {
    let cli = parse(&["chat", "hello"]);
    assert!(cli.prompt.is_none());
    assert!(matches!(cli.command, Some(Commands::Chat(_))));
}

#[test]
fn test_global_flags_work_bare() {
    let cli = parse(&["--mode", "plan"]);
    assert!(cli.command.is_none());
    assert!(cli.prompt.is_none());
    assert_eq!(cli.mode.as_deref(), Some("plan"));
}

#[test]
fn test_bare_prompt_with_globals() {
    let cli = parse(&["--mode", "plan", "fix it"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.prompt.as_deref(), Some("fix it"));
    assert_eq!(cli.mode.as_deref(), Some("plan"));
}

#[test]
fn test_cwd_resolves_as_project() {
    // Bare modes pass project=None; the resolver must accept the cwd.
    let proj = local_ai::core::projects::resolve_project(None).expect("cwd resolves");
    assert!(proj.folder_path.is_some());
}
