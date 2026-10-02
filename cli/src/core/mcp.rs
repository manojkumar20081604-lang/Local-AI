//! MCP tool servers — extensibility via JSON-RPC over stdio (Phase 4.2).
//!
//! External tools register with `local-ai --mcp filesystem --mcp github …`
//! and the model sees them alongside the built-ins (`list_project_files`,
//! `read_project_file`, …).
//!
//! Protocol (minimal, line-delimited JSON-RPC 2.0 over stdio — no SDK needed,
//! any `python3`/`node` script can be a server):
//!
//! ```text
//! → {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"client":"local-ai"}}
//! ← {"jsonrpc":"2.0","id":1,"result":{"server":"filesystem","version":"1"}}
//! → {"jsonrpc":"2.0","id":2,"method":"tools/list"}
//! ← {"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"read_file",…}]}}
//! → {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"…"}}}
//! ← {"jsonrpc":"2.0","id":3,"result":{…}}
//! ```
//!
//! Security: MCP writes/exec stay gated by plan/build mode + approval, and
//! every call's arguments pass the same denylist as shell commands
//! (plus an env-exfil screen: secret paths × network tools).
//! Reference servers in `mcp/` are read-only first.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use super::tools::ToolDefinition;

const PROTOCOL_VERSION: &str = "1";
const RESP_TIMEOUT: Duration = Duration::from_secs(10);

/// Validate `--mcp <name>`: filesystem-safe slug only.
pub fn validate_server_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 40 {
        anyhow::bail!("Invalid --mcp server '{}': must be 1-40 chars", name);
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("Invalid --mcp server '{}': alphanumeric, '-' and '_' only", name);
    }
    Ok(())
}

/// Locate `mcp/<name>/server.py`. Search order:
/// `$LOCAL_AI_MCP_DIR`, `./mcp` (cwd), `<exe-dir>/../mcp`, dev manifest dir.
pub fn server_path(name: &str) -> Result<PathBuf> {
    validate_server_name(name)?;
    let mut candidates = Vec::new();
    if let Ok(dir) = std::env::var("LOCAL_AI_MCP_DIR") {
        if !dir.is_empty() {
            candidates.push(PathBuf::from(dir).join(name).join("server.py"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("mcp").join(name).join("server.py"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("mcp").join(name).join("server.py"));
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("mcp").join(name).join("server.py"));
            }
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("mcp").join(name).join("server.py"));
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    anyhow::bail!(
        "MCP server '{}' not found — expected mcp/{}/server.py (set LOCAL_AI_MCP_DIR to override)",
        name,
        name
    )
}

fn python_bin() -> &'static str {
    if cfg!(target_os = "windows") { "python" } else { "python3" }
}

/// A live stdio session to one MCP server.
pub struct McpSession {
    name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    server_info: Value,
}

impl McpSession {
    /// Spawn `python3 <server.py> --root <project_root>` and initialize.
    pub fn connect(name: &str, project_root: &str) -> Result<Self> {
        let script = server_path(name)?;
        let mut child = Command::new(python_bin())
            .arg(&script)
            .arg("--root")
            .arg(project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Failed to launch MCP server '{}' (is python3 installed?)", name))?;
        let stdin = child.stdin.take().context("No stdin for MCP server")?;
        let stdout = child.stdout.take().context("No stdout for MCP server")?;
        let mut session = Self {
            name: name.to_string(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            server_info: Value::Null,
        };
        let info = session.request(
            "initialize",
            json!({"client": "local-ai", "protocol": PROTOCOL_VERSION}),
        )?;
        session.server_info = info;
        // Best-effort initialized notification (servers tolerate its absence).
        let _ = session.notify("notifications/initialized", json!({}));
        Ok(session)
    }

    pub fn server_name(&self) -> &str {
        &self.name
    }

    pub fn server_info(&self) -> &Value {
        &self.server_info
    }

    fn send_value(&mut self, v: &Value) -> Result<()> {
        let mut line = serde_json::to_string(v)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Read lines until the response with `id` arrives (or timeout).
    /// Server log lines (non-JSON) are skipped.
    fn read_response(&mut self, id: u64) -> Result<Value> {
        let deadline = Instant::now() + RESP_TIMEOUT;
        let mut line = String::new();
        loop {
            if Instant::now() > deadline {
                anyhow::bail!("MCP server '{}' timed out waiting for response {}", self.name, id);
            }
            line.clear();
            // Short poll so the timeout above stays responsive.
            let available = self.stdout.fill_buf().map(|b| b.len()).unwrap_or(0);
            if available == 0 {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            self.stdout.read_line(&mut line)?;
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue, // server log line — skip
            };
            if v.get("id").and_then(|x| x.as_u64()) != Some(id) {
                continue; // notification or stray — skip
            }
            if let Some(err) = v.get("error") {
                anyhow::bail!("MCP server '{}' error: {}", self.name, err);
            }
            return v.get("result").cloned().context("MCP response has no result");
        }
    }

    pub fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send_value(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        self.read_response(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send_value(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// `tools/list` → raw MCP tool descriptors.
    pub fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let result = self.request("tools/list", json!({}))?;
        let tools = result.get("tools").and_then(|t| t.as_array()).cloned().unwrap_or_default();
        let mut out = Vec::new();
        for t in tools {
            out.push(McpTool {
                name: t.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                description: t.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
                parameters: t.get("parameters").cloned().unwrap_or(json!({"type": "object"})),
            });
        }
        Ok(out)
    }

    /// `tools/call` → result value (or error when the server reports one).
    pub fn call_tool(&mut self, tool: &str, arguments: &Value) -> Result<Value> {
        self.request("tools/call", json!({"name": tool, "arguments": arguments}))
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// One tool advertised by an MCP server.
#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl McpTool {
    /// Exposed to the model as `mcp__<server>__<tool>` (collision-free).
    pub fn qualified_name(&self, server: &str) -> String {
        format!("mcp__{}__{}", server, self.name)
    }

    pub fn to_definition(&self, server: &str) -> ToolDefinition {
        ToolDefinition {
            type_: "function".into(),
            function: super::tools::ToolFunction {
                name: self.qualified_name(server),
                description: format!("[mcp:{}] {}", server, self.description),
                parameters: self.parameters.clone(),
            },
        }
    }
}

/// Split a qualified model tool name back into `(server, tool)`.
pub fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server, tool))
}

// ---------------------------------------------------------------------------
// Security screening (kill-switch for external tools)
// ---------------------------------------------------------------------------

/// Tool-name hints that a call may write or execute.
fn looks_write_like(tool: &str) -> bool {
    let t = tool.to_lowercase();
    ["write", "create", "delete", "remove", "exec", "run", "shell", "apply", "commit", "push", "publish", "deploy", "update", "set_", "save"]
        .iter()
        .any(|hint| t.contains(hint))
}

fn looks_network_like(server: &str, tool: &str) -> bool {
    let hay = format!("{} {}", server, tool).to_lowercase();
    ["github", "http", "fetch", "publish", "post", "issue", "pr_", "webhook", "curl", "request"]
        .iter()
        .any(|hint| hay.contains(hint))
}

fn looks_secret_like(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains(".env")
        || t.contains(".pem")
        || t.contains("id_rsa")
        || t.contains("id_ed25519")
        || t.contains("aws_secret")
        || t.contains("akid")
        || t.contains("api_key")
        || t.contains("secret")
}

/// Gate one MCP call. Errors when the call must not proceed:
/// - write/exec-like tools in plan mode (read-only),
/// - shell-dangerous argument strings without `approve_dangerous`,
/// - secret paths × network tools (env exfil) without `approve_dangerous`.
pub fn mcp_call_allowed(
    server: &str,
    tool: &str,
    arguments: &Value,
    approve_dangerous: bool,
) -> Result<()> {
    if looks_write_like(tool) && crate::core::config::is_plan_mode() {
        anyhow::bail!(
            "Plan mode is active — MCP tool '{}' (server '{}') may write/execute. Switch to --mode build to allow.",
            tool,
            server
        );
    }
    let args_text = arguments.to_string();
    // Reuse the shell kill-switch on string arguments (covers `curl … | sh` smuggled into args).
    if let Some(args) = arguments.as_object() {
        for (k, v) in args {
            if let Some(s) = v.as_str() {
                if super::agents::is_dangerous_command(s).is_some() && !approve_dangerous {
                    anyhow::bail!(
                        "MCP argument '{}' blocked by denylist — re-run with --approve dangerous to allow",
                        k
                    );
                }
            }
        }
    }
    if looks_secret_like(&args_text) && looks_network_like(server, tool) && !approve_dangerous {
        anyhow::bail!(
            "MCP call '{}' on '{}' looks like env exfil (secret path × network tool) — re-run with --approve dangerous to allow",
            tool,
            server
        );
    }
    Ok(())
}
