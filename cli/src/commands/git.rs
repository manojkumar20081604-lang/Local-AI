//! `local-ai git` — read-only git intelligence for the developer agent.
//!
//! All subcommands are read-only and allowed in plan mode. Commits arrive
//! separately in Phase 1.3 behind the build-mode gate.

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;

use crate::core::{git as core_git, projects};

#[derive(Parser)]
pub struct GitArgs {
    #[command(subcommand)]
    pub command: GitCommands,
}

#[derive(Subcommand)]
pub enum GitCommands {
    /// Show branch + staged/unstaged/untracked files
    Status {
        /// Project id/name/path (default: current dir)
        #[arg(long)]
        project: Option<String>,
    },
    /// Show unstaged (or --staged) diff, optionally for one file
    Diff {
        #[arg(long)]
        project: Option<String>,
        /// Show staged diff instead of unstaged
        #[arg(long)]
        staged: bool,
        /// Limit to one repo-relative file
        #[arg(long)]
        file: Option<String>,
    },
    /// Show recent commits (newest first)
    Log {
        #[arg(long)]
        project: Option<String>,
        /// Number of commits (default 10, max 200)
        #[arg(long, short = 'n', default_value = "10")]
        number: u32,
    },
    /// Blame a file (who changed each line), optionally -L range
    Blame {
        #[arg(long)]
        project: Option<String>,
        /// Repo-relative file path
        #[arg(long)]
        file: String,
        /// Line range N or N,M (e.g. 10,40)
        #[arg(long)]
        lines: Option<String>,
    },
    /// List branches (current marked with *)
    Branches {
        #[arg(long)]
        project: Option<String>,
    },
    /// List stashes
    Stash {
        #[arg(long)]
        project: Option<String>,
    },
    /// Restore the newest local-ai checkpoint (stash pop; build-gated)
    Rollback {
        #[arg(long)]
        project: Option<String>,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Commit staged changes (build-gated; preview with --dry-run)
    Commit {
        #[arg(long)]
        project: Option<String>,
        /// Commit message (default: auto-generated conventional message)
        #[arg(long, short = 'm')]
        message: Option<String>,
        /// Preview the changeset + proposed message without committing
        #[arg(long)]
        dry_run: bool,
        /// Run this test command first; abort on failure
        #[arg(long)]
        test_cmd: Option<String>,
        /// Skip the approval prompt
        #[arg(long)]
        yes: bool,
    },
}

pub async fn handle(args: GitArgs) -> Result<()> {
    // Intentionally NO require_build_mode — everything here is read-only.
    match args.command {
        GitCommands::Status { project } => {
            let proj = projects::resolve_project(project)?;
            let root = core_git::project_root(&proj)?;
            let st = core_git::status(&root)?;
            println!(
                "{} {} {}",
                style("branch:").dim(),
                style(&st.branch).cyan().bold(),
                if st.is_clean() {
                    style("(clean)").green().to_string()
                } else {
                    String::new()
                }
            );
            if !st.staged.is_empty() {
                println!("\n{}", style("Staged:").bold());
                for c in &st.staged {
                    println!("  {} {}", style(&c.xy).green(), c.path);
                }
            }
            if !st.unstaged.is_empty() {
                println!("\n{}", style("Unstaged:").bold());
                for c in &st.unstaged {
                    println!("  {} {}", style(&c.xy).yellow(), c.path);
                }
            }
            if !st.untracked.is_empty() {
                println!("\n{}", style("Untracked:").bold());
                for p in &st.untracked {
                    println!("  {} {}", style("??").red(), p);
                }
            }
            if st.is_clean() {
                println!("nothing to commit, working tree clean");
            }
        }
        GitCommands::Diff { project, staged, file } => {
            let proj = projects::resolve_project(project)?;
            let root = core_git::project_root(&proj)?;
            let out = core_git::diff(&root, staged, file.as_deref())?;
            if out.trim().is_empty() {
                println!(
                    "{}",
                    style(if staged {
                        "No staged changes"
                    } else {
                        "No unstaged changes"
                    })
                    .dim()
                );
            } else {
                print!("{}", out);
            }
        }
        GitCommands::Log { project, number } => {
            let proj = projects::resolve_project(project)?;
            let root = core_git::project_root(&proj)?;
            let commits = core_git::log(&root, number)?;
            if commits.is_empty() {
                println!("{}", style("No commits yet").dim());
                return Ok(());
            }
            for c in &commits {
                println!(
                    "{} {} {} {}",
                    style(&c.short).yellow(),
                    style(&c.date).dim(),
                    style(&c.author).cyan(),
                    c.message
                );
            }
        }
        GitCommands::Blame { project, file, lines } => {
            let proj = projects::resolve_project(project)?;
            let root = core_git::project_root(&proj)?;
            let out = core_git::blame(&root, &file, lines.as_deref())?;
            print!("{}", out);
        }
        GitCommands::Branches { project } => {
            let proj = projects::resolve_project(project)?;
            let root = core_git::project_root(&proj)?;
            for b in core_git::branches(&root)? {
                if b.current {
                    println!("{} {}", style("*").green().bold(), style(&b.name).bold());
                } else {
                    println!("  {}", b.name);
                }
            }
        }
        GitCommands::Stash { project } => {
            let proj = projects::resolve_project(project)?;
            let root = core_git::project_root(&proj)?;
            let entries = core_git::stash_list(&root)?;
            if entries.is_empty() {
                println!("{}", style("No stashes").dim());
            } else {
                for e in &entries {
                    println!("{}", e);
                }
            }
        }
        GitCommands::Commit { project, message, dry_run, test_cmd, yes } => {
            handle_commit(project, message, dry_run, test_cmd, yes).await?;
        }
        GitCommands::Rollback { project, yes } => {
            // Unlike the rest of `git` (read-only), rollback rewrites the tree.
            crate::core::config::require_build_mode("git rollback")?;
            let proj = projects::resolve_project(project)?;
            if !yes {
                let ok = dialoguer::Confirm::new()
                    .with_prompt("Restore the newest local-ai checkpoint (uncommitted work may conflict)?")
                    .default(false)
                    .interact_opt()?;
                if !ok.unwrap_or(false) {
                    println!("{} rollback cancelled — nothing changed", style("✗").red());
                    return Ok(());
                }
            }
            match crate::core::git::rollback_checkpoint(&proj) {
                Ok(msg) => println!("{} {}", style("✓").green(), msg),
                Err(e) => {
                    eprintln!("{} {}", style("✗").red(), e);
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

async fn handle_commit(
    project: Option<String>,
    message: Option<String>,
    dry_run: bool,
    test_cmd: Option<String>,
    yes: bool,
) -> Result<()> {
    use crate::core::fs as core_fs;

    let proj = projects::resolve_project(project)?;
    let root = core_git::project_root(&proj)?;
    let st = core_git::status(&root)?;
    if st.staged.is_empty() {
        if st.unstaged.is_empty() && st.untracked.is_empty() {
            println!("{}", style("Nothing to commit, working tree clean").dim());
        } else {
            anyhow::bail!("Nothing staged — stage changes first (git add), then commit");
        }
        return Ok(());
    }
    let stats = core_git::diff_stat_staged(&root)?;
    let msg = message.unwrap_or_else(|| core_git::propose_message(&st.staged, &stats));

    // AI CHANGESET preview (read-only — allowed in plan mode).
    let total_added: u64 = stats.iter().map(|s| s.added).sum();
    let total_deleted: u64 = stats.iter().map(|s| s.deleted).sum();
    println!("{}", style("AI CHANGESET").bold());
    println!("  Files changed: {}", st.staged.len());
    println!("  Lines: {} added, {} removed", style(total_added).green(), style(total_deleted).red());
    for s in &stats {
        println!("    {} +{} -{}", s.path, s.added, s.deleted);
    }
    // Untracked files are NOT committed (staged-only semantics) — say so.
    if !st.untracked.is_empty() {
        println!("  {} {} untracked (not included — `git add` to stage)", style("→").dim(), st.untracked.len());
    }
    println!("\n  Commit message:\n    {}", style(&msg).cyan());
    if !core_git::is_conventional(&msg) {
        eprintln!("  {} message is not conventional (`type(scope): subject`) — prefer: feat({}): …",
            style("!").yellow(), st.branch);
    }
    let changed_paths: Vec<String> = st.staged.iter().map(|c| c.path.clone()).collect();
    let outside = core_git::message_mentions_outside_changeset(&msg, &changed_paths);
    if !outside.is_empty() {
        eprintln!("  {} message mentions files outside the changeset: {}",
            style("⚠").yellow(), outside.join(", "));
    }

    if dry_run || crate::core::config::is_plan_mode() {
        if crate::core::config::is_plan_mode() && !dry_run {
            eprintln!("\n  {} plan mode — preview only. Switch to build to commit.", style("→").dim());
        } else {
            eprintln!("\n  {} dry-run — nothing committed. Re-run with --yes (or confirm) to commit.", style("→").dim());
        }
        return Ok(());
    }
    crate::core::config::require_build_mode("git commit")?;

    // Test gate: red tests block the commit.
    if let Some(cmd) = test_cmd {
        println!("\n{} running test gate `{}`…", style("→").dim(), cmd);
        let result = core_fs::run_project_command(&proj, &cmd)?;
        if !result.success {
            eprintln!("{} tests failed (exit {}) — commit aborted",
                style("✗").red(), result.exit_code.unwrap_or(1));
            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
            }
            anyhow::bail!("test gate red — fix failures, then commit");
        }
        println!("{} tests green", style("✓").green());
    }

    if !yes {
        let ok = dialoguer::Confirm::new()
            .with_prompt(format!("Commit {} file(s) with message \"{}\"?", st.staged.len(), msg.lines().next().unwrap_or("")))
            .default(false)
            .interact_opt()?;
        if !ok.unwrap_or(false) {
            println!("{} rejected by user", style("✗").red());
            return Ok(());
        }
    }
    let hash = core_git::commit(&root, &msg)?;
    println!("\n{} committed {} — {} file(s), +{}/-{}",
        style("✓").green(), style(&hash[..7.min(hash.len())]).yellow(),
        st.staged.len(), total_added, total_deleted);
    // Phase 3.1: journal the commit into project memory (best-effort, never fails the commit).
    // Stores file list + stack, never secret contents (redacted at save time).
    match crate::core::memory::update_project_memory_after_commit(&proj, &changed_paths, &hash) {
        Ok(p) => eprintln!("{} memory: journalled commit → {}", style("→").dim(), p.display()),
        Err(e) => eprintln!("{} memory journal skipped: {}", style("!").yellow(), e),
    }
    Ok(())
}
