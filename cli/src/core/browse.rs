//! Browser agent — fetch pages, extract code blocks, cite URLs (Phase 4.3).
//!
//! `web_fetch(url)` stays the read primitive (10s timeout, truncation).
//! `browse` builds on it: seed URLs (search results or `--seed-url`) are
//! fetched, fenced code blocks are extracted, and every answer cites its
//! URLs. Browsing is never silent: each fetched URL is logged (the CLI
//! prints every fetch; `--show-context` adds snippets).

use anyhow::{Context, Result};

/// One fetched page with extracted code.
#[derive(Debug, Clone)]
pub struct BrowsedPage {
    pub url: String,
    pub title: String,
    pub code_blocks: Vec<CodeBlock>,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub lang: String,
    pub code: String,
    pub source_url: String,
}

/// Fetch raw text with the shared browser settings (10s, Local-AI UA).
pub async fn fetch_url(url: &str, max_chars: usize) -> Result<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!("Invalid URL '{}': must start with http:// or https://", url);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client
        .get(url)
        .header("User-Agent", "Local-AI/0.1 (browser)")
        .send()
        .await
        .context("Failed to fetch URL")?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} for {}", resp.status(), url);
    }
    let text = resp.text().await.context("Failed to read body")?;
    Ok(truncate_chars(&text, max_chars))
}

fn truncate_chars(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &s[..end])
}

/// Extract `href="…"` links, resolved against `base`. Fragments dropped,
/// `mailto:`/`javascript:` skipped, order preserved, deduped.
pub fn extract_links(html: &str, base: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let lower = html.to_lowercase();
    let mut idx = 0;
    while let Some(pos) = lower[idx..].find("href") {
        let abs = idx + pos;
        // Skip `href`, optional whitespace, and the `=` sign.
        let mut rest = html[abs + 4..].trim_start();
        if let Some(stripped) = rest.strip_prefix('=') {
            rest = stripped.trim_start();
        }
        let quote = rest.chars().next();
        let raw = match quote {
            Some('"') | Some('\'') => {
                let q = quote.unwrap();
                let after = &rest[1..];
                match after.find(q) {
                    Some(end) => &after[..end],
                    None => {
                        idx = abs + 4;
                        continue;
                    }
                }
            }
            _ => {
                // Unquoted href — take to whitespace or `>`.
                let token: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != '>').collect();
                let token = token.trim_end_matches('>').to_string();
                if token.is_empty() {
                    idx = abs + 4;
                    continue;
                }
                // Leak-free: process inline then continue.
                let resolved = resolve_url(&token, base);
                idx = abs + 4;
                if let Some(u) = resolved {
                    if seen.insert(u.clone()) {
                        out.push(u);
                    }
                }
                continue;
            }
        };
        idx = abs + 4;
        if let Some(u) = resolve_url(raw, base) {
            if seen.insert(u.clone()) {
                out.push(u);
            }
        }
    }
    out
}

fn resolve_url(raw: &str, base: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') {
        return None;
    }
    let lower = raw.to_lowercase();
    if lower.starts_with("mailto:") || lower.starts_with("javascript:") || lower.starts_with("data:") {
        return None;
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(raw.split('#').next().unwrap_or(raw).to_string());
    }
    if raw.starts_with("//") {
        let scheme = if base.starts_with("https") { "https:" } else { "http:" };
        return Some(format!("{}{}", scheme, raw.split('#').next().unwrap_or(raw)));
    }
    // Path-absolute or relative: join with base origin + dir.
    let (origin, dir) = split_origin_dir(base);
    let path = if raw.starts_with('/') {
        raw.to_string()
    } else if dir.is_empty() {
        format!("/{}", raw)
    } else {
        format!("{}/{}", dir, raw)
    };
    let normalized = normalize_path(&path);
    Some(format!("{}{}", origin, normalized.split('#').next().unwrap_or(&normalized)))
}

fn split_origin_dir(base: &str) -> (String, String) {
    // `https://host[:port]/a/b` → (`https://host[:port]`, `/a`)
    let without_scheme = match base.find("://") {
        Some(p) => &base[p + 3..],
        None => base,
    };
    let scheme = &base[..base.find("://").map(|p| p + 3).unwrap_or(0)];
    let (host, path) = match without_scheme.find('/') {
        Some(i) => (&without_scheme[..i], &without_scheme[i..]),
        None => (without_scheme, "/"),
    };
    let dir = path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    (format!("{}{}", scheme, host), dir)
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Extract fenced code blocks (```lang … ```). Unclosed fences are kept
/// (some docs truncate); inline single-backtick spans are ignored.
pub fn extract_code_blocks(text: &str, source_url: &str) -> Vec<CodeBlock> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let (lang, body_start) = match after.find('\n') {
            Some(nl) => (after[..nl].trim().to_string(), start + 3 + nl + 1),
            None => (String::new(), start + 3),
        };
        let body = &rest[body_start..];
        let (code, next) = match body.find("```") {
            Some(end) => (&body[..end], body_start + end + 3),
            None => (body, rest.len()),
        };
        let code = code.trim().to_string();
        if !code.is_empty() {
            out.push(CodeBlock { lang, code, source_url: source_url.to_string() });
        }
        if next >= rest.len() {
            break;
        }
        rest = &rest[next..];
    }
    out
}

/// Page `<title>` (empty when absent).
pub fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    let (Some(s), Some(e)) = (lower.find("<title"), lower.find("</title>")) else {
        return String::new();
    };
    let open_end = match html[s..].find('>') {
        Some(i) => s + i + 1,
        None => return String::new(),
    };
    if open_end >= e {
        return String::new();
    }
    html[open_end..e].trim().to_string()
}

/// Fetch `urls` (deduped, capped at `max_pages`), extract code blocks.
/// `fetch` is injectable so tests run against a local fixture server.
pub async fn browse_pages<F, Fut>(urls: &[String], max_pages: usize, fetch: F) -> Vec<BrowsedPage>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let mut seen = std::collections::HashSet::new();
    let mut pages = Vec::new();
    for url in urls {
        if pages.len() >= max_pages {
            break;
        }
        if !seen.insert(url.clone()) {
            continue;
        }
        let text = match fetch(url.clone()).await {
            Ok(t) => t,
            Err(e) => {
                pages.push(BrowsedPage {
                    url: url.clone(),
                    title: String::new(),
                    code_blocks: Vec::new(),
                    snippet: format!("fetch failed: {}", e),
                });
                continue;
            }
        };
        let code_blocks = extract_code_blocks(&text, url);
        pages.push(BrowsedPage {
            title: extract_title(&text),
            snippet: truncate_chars(&text, 500),
            url: url.clone(),
            code_blocks,
        });
    }
    pages
}

/// Render fetched pages with URL citations (`[source: url]`).
pub fn render_with_citations(pages: &[BrowsedPage]) -> String {
    let mut out = String::new();
    for page in pages {
        out.push_str(&format!("## [source: {}]\n", page.url));
        if !page.title.is_empty() {
            out.push_str(&format!("Title: {}\n", page.title));
        }
        if page.code_blocks.is_empty() {
            out.push_str(&format!("{}\n\n", page.snippet));
        } else {
            for block in &page.code_blocks {
                out.push_str(&format!("```{}\n{}\n```\n[source: {}]\n\n", block.lang, block.code, block.source_url));
            }
        }
    }
    out
}

/// Seed URLs from a DuckDuckGo HTML search (live network). Pure scraping of
/// the HTML endpoint — no API key. Returns up to `limit` result URLs.
pub async fn ddg_seed_urls(query: &str, limit: usize) -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let html = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Local-AI/0.1 (browser)")
        .send()
        .await
        .context("Search request failed")?
        .text()
        .await
        .context("Failed to read search body")?;
    // DDG wraps results as `/l/?uddg=<urlencoded>` links plus direct hrefs.
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for link in extract_links(&html, "https://html.duckduckgo.com") {
        let decoded = decode_uddg(&link);
        if (decoded.starts_with("http://") || decoded.starts_with("https://")) && seen.insert(decoded.clone()) {
            out.push(decoded);
        }
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn decode_uddg(link: &str) -> String {
    // `/l/?uddg=https%3A%2F%2F…&rut=…` → the target URL.
    let Some(q) = link.split('?').nth(1) else {
        return link.to_string();
    };
    for pair in q.split('&') {
        if let Some(v) = pair.strip_prefix("uddg=") {
            return url_decode(v);
        }
    }
    link.to_string()
}

fn url_decode(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as char);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { ' ' } else { bytes[i] as char });
        i += 1;
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
