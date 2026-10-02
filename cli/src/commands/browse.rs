//! `local-ai browse` — browser agent over `web_fetch` (Phase 4.3).
//!
//! ```bash
//! local-ai browse "stripe checkout api" --max-pages 5
//! local-ai browse "tokio spawn" --seed-url https://docs.rs/tokio --max-pages 3 --show-context
//! ```
//!
//! Seeds come from a live web search by default (`--seed-url` pins them for
//! reproducibility/offline use). Every fetched URL is printed — browsing is
//! never silent; `--show-context` adds per-page snippets.

use anyhow::Result;
use clap::Parser;
use console::style;

use crate::core::browse as core_browse;

#[derive(Parser)]
pub struct BrowseArgs {
    /// What to research, e.g. "stripe checkout api"
    pub query: String,

    /// Max pages to fetch (default 5, max 20)
    #[arg(long, default_value = "5")]
    pub max_pages: usize,

    /// Pin seed URLs instead of searching (repeatable, offline-friendly)
    #[arg(long)]
    pub seed_url: Vec<String>,

    /// Show per-page snippets in addition to code blocks
    #[arg(long)]
    pub show_context: bool,
}

pub async fn handle(args: BrowseArgs) -> Result<()> {
    let max_pages = args.max_pages.clamp(1, 20);

    // Seeds: pinned URLs or a live search (search itself is logged).
    let seeds = if args.seed_url.is_empty() {
        println!("{} searching for {:?}…", style("🔍").dim(), args.query);
        match core_browse::ddg_seed_urls(&args.query, max_pages).await {
            Ok(urls) if !urls.is_empty() => urls,
            Ok(_) => anyhow::bail!("No search results for {:?} — try --seed-url <url>", args.query),
            Err(e) => anyhow::bail!("Search failed ({}). Pass explicit --seed-url <url> instead.", e),
        }
    } else {
        args.seed_url.clone()
    };
    println!("{} {} seed URL(s):", style("→").dim(), seeds.len().min(max_pages));
    for url in seeds.iter().take(max_pages) {
        println!("  {}", style(url).cyan());
    }

    // Fetch with a visible log per URL (never silent browsing).
    let pages = core_browse::browse_pages(&seeds, max_pages, |url| async move {
        println!("{} fetching {}", style("↓").dim(), url);
        let text = core_browse::fetch_url(&url, 8000).await?;
        Ok(text)
    })
    .await;

    let mut total_blocks = 0;
    for page in &pages {
        total_blocks += page.code_blocks.len();
        if page.code_blocks.is_empty() && page.snippet.starts_with("fetch failed") {
            eprintln!("  {} {} — {}", style("✗").red(), page.url, page.snippet);
        } else {
            println!(
                "  {} {} — {}{}",
                style("✓").green(),
                page.url,
                if page.title.is_empty() { "(no title)".to_string() } else { format!("{:?}", page.title) },
                if args.show_context { format!("\n    {}", page.snippet.lines().next().unwrap_or("")) } else { String::new() }
            );
        }
    }

    println!("\n{}", style(format!("BROWSED {:?} — {} pages, {} code blocks", args.query, pages.len(), total_blocks)).bold());
    print!("{}", core_browse::render_with_citations(&pages));
    Ok(())
}
