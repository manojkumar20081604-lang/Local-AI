use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::projects::Project;
use super::fs as core_fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub type_: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: Option<String>,
    pub name: String,
    pub arguments: String, // JSON string
    pub parsed_args: Value,
}

impl ToolCall {
    pub fn new(name: String, args: Value) -> Self {
        Self {
            id: Some(format!("call_{}", rand_string(8))),
            name,
            arguments: args.to_string(),
            parsed_args: args,
        }
    }
}

fn rand_string(n: usize) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    format!("{:x}", h.finish())[..n.min(8)].to_string()
}

/// Project-aware tools that the model can call instead of hallucinating.
/// Includes browser + terminal access for reading all necessary files (as requested).
pub fn project_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            type_: "function".into(),
            function: ToolFunction {
                name: "list_project_files".into(),
                description: "List all real files in the attached project (authoritative inventory). Use this before claiming a file exists.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolDefinition {
            type_: "function".into(),
            function: ToolFunction {
                name: "read_project_file".into(),
                description: "Read a real file from the project. Path must be relative, e.g. 'src/main.rs' or 'cli/src/main.rs'. Returns file content or error if not found. Use this to verify file exists before discussing it.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Relative file path, e.g. 'src/main.rs'"}
                    },
                    "required": ["path"]
                }),
            },
        },
        ToolDefinition {
            type_: "function".into(),
            function: ToolFunction {
                name: "search_project".into(),
                description: "Search for a symbol or keyword in project files (grep). Returns file paths and snippets where found. Use to verify a function/symbol exists before claiming it.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Symbol or keyword to search, e.g. 'getProjects' or 'AuthService'"}
                    },
                    "required": ["query"]
                }),
            },
        },
        ToolDefinition {
            type_: "function".into(),
            function: ToolFunction {
                name: "exec".into(),
                description: "Execute a terminal command inside the project (read all necessary files). Examples: 'ls -R', 'cat src/main.rs', 'find . -name \"*.rs\" | head -20', 'grep -rn \"TODO\" --include=\"*.rs\"', 'wc -l src/*.rs'. Use this to explore project via terminal when read_project_file is insufficient. In plan mode, only read-only commands (ls/cat/find/grep/head) are allowed — write/build commands (rm/cargo build/npm build) are blocked.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "Shell command to run, e.g. 'ls -la' or 'grep -rn \"auth\" src/'"}
                    },
                    "required": ["command"]
                }),
            },
        },
        ToolDefinition {
            type_: "function".into(),
            function: ToolFunction {
                name: "web_fetch".into(),
                description: "Browser access — fetch a web page by URL and return its text content (markdown/text). Use for docs, crates.io, GitHub, API references, or any web resource. Example: 'https://doc.rust-lang.org/book/'. Requires network; returns truncated content (4000 chars).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "HTTP/HTTPS URL to fetch, e.g. 'https://example.com'"},
                        "max_chars": {"type": "integer", "description": "Max chars to return (default 4000, max 8000)"}
                    },
                    "required": ["url"]
                }),
            },
        },
    ]
}

/// Extended tools including browser + terminal (for --tools mode)
pub fn browser_tools() -> Vec<ToolDefinition> {
    project_tools()
}

pub fn tools_json() -> Value {
    serde_json::to_value(project_tools()).unwrap_or(json!([]))
}

pub fn supports_tools_for_model(model: &str) -> bool {
    // Small models (<1b) rarely support tools reliably; qwen2.5:0.5b does not.
    // Known good: llama3.1:8b, qwen2.5:7b, mistral:7b, qwen3:8b, gpt-oss, etc.
    let lower = model.to_lowercase();
    if lower.contains("0.5b") || lower.contains("1b") && lower.contains("llama3.2") {
        return false; // too small
    }
    // Assume larger models support tools
    true
}

/// Execute a tool call locally (deterministic, no LLM) — now async for web_fetch
pub async fn execute_tool(project: &Project, tool_call: &ToolCall) -> Result<Value> {
    match tool_call.name.as_str() {
        "list_project_files" => {
            let files = core_fs::list_project_files(project)?;
            let list: Vec<Value> = files.iter().map(|f| json!({"path": f.path, "is_directory": f.is_directory})).collect();
            Ok(json!({"files": list, "total": files.len()}))
        }
        "read_project_file" => {
            let path = tool_call.parsed_args.get("path").and_then(|v| v.as_str()).context("Missing 'path'")?;
            match core_fs::read_project_file(project, path) {
                Ok(content) => {
                    let snippet = if content.len() > 4000 {
                        let mut end = 4000;
                        while end > 0 && !content.is_char_boundary(end) { end -= 1; }
                        content[..end].to_string() + "\n...[truncated]"
                    } else { content };
                    Ok(json!({"path": path, "content": snippet, "exists": true}))
                }
                Err(e) => Ok(json!({"path": path, "error": e.to_string(), "exists": false})),
            }
        }
        "search_project" => {
            let query = tool_call.parsed_args.get("query").and_then(|v| v.as_str()).context("Missing 'query'")?;
            let files = core_fs::list_project_files(project)?;
            let mut hits = Vec::new();
            for f in files.iter().filter(|f| !f.is_directory).take(100) {
                if let Ok(content) = core_fs::read_project_file(project, &f.path) {
                    if content.to_lowercase().contains(&query.to_lowercase()) {
                        // Find snippet
                        let idx = content.to_lowercase().find(&query.to_lowercase()).unwrap_or(0);
                        let start = idx.saturating_sub(80);
                        let end = (idx + query.len() + 80).min(content.len());
                        let snippet = content[start..end].replace('\n', " ");
                        hits.push(json!({"path": f.path, "snippet": snippet}));
                        if hits.len() >= 5 { break; }
                    }
                }
            }
            Ok(json!({"query": query, "hits": hits, "found": !hits.is_empty()}))
        }
        "exec" => {
            let cmd = tool_call.parsed_args.get("command").and_then(|v| v.as_str()).context("Missing 'command'")?;
            // In plan mode, block write/build commands — cross-platform (Unix + Windows)
            if crate::core::config::is_plan_mode() {
                let lower = cmd.to_lowercase();
                let blocked = ["rm ", "rm\t", "rm\n", "cargo build", "cargo run", "npm run build", "npm run dev", "npm install", "yarn build", "pnpm build", "make ", "cmake", "docker build", "go build", "touch ", "mkdir ", "mv ", "cp ", "chmod ", "sudo ", ">", ">>", "truncate", "dd ", "del ", "del\t", "rmdir", "powershell", "pwsh", "format ", "mkfs", "shutdown", "reboot", "diskpart", "xcopy", "robocopy", "move ", "copy "];
                for pat in blocked {
                    if lower.contains(pat) {
                        anyhow::bail!("Plan mode is active — exec '{}' is blocked (write/build). Switch to --mode build to allow.", cmd);
                    }
                }
            }
            let result = core_fs::run_project_command(project, cmd)?;
            let truncate_safe = |s: &str, n: usize| -> String {
                if s.len() <= n { return s.to_string(); }
                let mut end = n;
                while end > 0 && !s.is_char_boundary(end) { end -= 1; }
                s[..end].to_string() + "\n...[truncated]"
            };
            let stdout = truncate_safe(&result.stdout, 4000);
            let stderr = truncate_safe(&result.stderr, 1000);
            Ok(json!({"command": cmd, "stdout": stdout, "stderr": stderr, "success": result.success, "exit_code": result.exit_code}))
        }
        "web_fetch" => {
            let url = tool_call.parsed_args.get("url").and_then(|v| v.as_str()).context("Missing 'url'")?;
            let max_chars = tool_call.parsed_args.get("max_chars").and_then(|v| v.as_u64()).unwrap_or(4000) as usize;
            // Allow browser fetch even in plan mode (read-only)
            if !url.starts_with("http://") && !url.starts_with("https://") {
                anyhow::bail!("Invalid URL '{}': must start with http:// or https://", url);
            }
            let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build()?;
            let resp = client.get(url)
                .header("User-Agent", "Local-AI/0.1 (browser)")
                .send().await
                .context("Failed to fetch URL")?;
            let status = resp.status();
            let headers = resp.headers().clone();
            if !status.is_success() {
                anyhow::bail!("HTTP {} for {}", status, url);
            }
            let text = resp.text().await.context("Failed to read body")?;
            let snippet = if text.len() > max_chars {
                let mut end = max_chars;
                while end > 0 && !text.is_char_boundary(end) { end -= 1; }
                text[..end].to_string() + "\n...[truncated]"
            } else { text };
            Ok(json!({"url": url, "content": snippet, "status": status.as_u16(), "content_type": headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string()}))
        }
        _ => anyhow::bail!("Unknown tool: {}", tool_call.name),
    }
}

/// Sync wrapper for tests and non-async callers (blocks on current runtime)
pub fn execute_tool_sync(project: &Project, tool_call: &ToolCall) -> Result<Value> {
    // For sync callers outside tokio, create a runtime; for inside tokio, use block_in_place if possible
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // Inside tokio — we are already in runtime, use the handle to block? Instead use futures::executor::block_on via handle? Simpler: use tokio::task::block_in_place if available
        // Fallback: spawn blocking task and block_on
        // Use handle.block_on(async) via spawn? But we are inside runtime, can't block_on directly.
        // Use a new runtime in a thread
        let project_clone = project.clone();
        let call_clone = tool_call.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let res = rt.block_on(execute_tool(&project_clone, &call_clone));
            let _ = tx.send(res);
        });
        rx.recv().unwrap()
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(execute_tool(project, tool_call))
    }
}

/// Parse tool calls from a response that may contain JSON tool_calls (for testing)
pub fn parse_tool_calls_from_text(text: &str) -> Vec<ToolCall> {
    // Try to find <tool_call> blocks or JSON tool calls embedded
    // For now, just look for function call patterns
    Vec::new()
}
