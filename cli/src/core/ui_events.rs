//! Agent event bus — the TUI consumes these instead of driving the engine.
//!
//! The engine (runner/agent/debug) emits [`UiEvent`]s and keeps working
//! exactly as before: with no subscriber the bus is a no-op and all output
//! goes to the terminal. `local-ai tui` subscribes, takes over the screen,
//! and routes approvals back through oneshot responders.
//!
//! Two globals coordinate the takeover:
//! - [`set_ui_takeover`] — the TUI sets this; engine `println!`s reroute here.
//! - [`set_approval_hook`] — the TUI installs a channel; [`request_approval`]
//!   delivers permission modals instead of blocking on stdin.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};
use tokio::sync::{broadcast, mpsc, oneshot};

/// What the user picked in a permission modal (mirrors the CLI tiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChoice {
    Once,
    Session,
    Reject,
}

/// One permission request: prompt + optional diff preview lines.
pub struct ApprovalRequest {
    pub prompt: String,
    pub diff: Vec<String>,
    pub respond: oneshot::Sender<ApprovalChoice>,
}

/// Events the engine emits; the TUI renders them into panels/character.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Faint log line (mirrors a terminal println when no TUI is attached).
    Log { line: String },
    /// Warning line (mirrors eprintln).
    Warn { line: String },
    /// A run started: goal + step count + budget.
    AgentStart { goal: String, steps: usize, max_steps: u32 },
    /// Plan graph created (step ids + titles + kinds).
    PlanCreated { steps: Vec<(String, String, String)> },
    /// A plan step began executing.
    StepStart { id: String, title: String, kind: String },
    /// A plan step finished (`done|pass|fail|manual|refused|blocked|…`).
    StepDone { id: String, status: String },
    /// Tool-style activity for the feed (`researcher/inspect`, `tester/test`…).
    ToolActivity { agent: String, action: String, detail: String, status: String },
    /// Streaming model chunk (appended to the active answer).
    ModelChunk { text: String },
    /// Testsuite outcome for the dashboard.
    TestResult { step: String, passed: bool, summary: String },
    /// Proposed edits with rendered diff lines (diff viewer panel).
    EditProposed { step: String, files: Vec<String>, diff: Vec<String> },
    /// Edits landed on disk.
    EditApplied { step: String, applied: Vec<String> },
    /// Reviewer verdict.
    Review { passed: bool, summary: String },
    /// A direct answer started (Q&A path, not the plan loop).
    AnswerStart { query: String },
    /// A direct answer finished streaming (full text for history).
    ModelDone { full: String },
    /// The effective model changed (resolve, /model, fallback) — header follows.
    ModelSwitch { model: String, provider: String },
    /// Run finished.
    AgentComplete { green: bool, summary: String },
}

fn bus() -> &'static broadcast::Sender<UiEvent> {
    static BUS: OnceLock<broadcast::Sender<UiEvent>> = OnceLock::new();
    BUS.get_or_init(|| broadcast::channel(512).0)
}

/// Subscribe to engine events (the TUI holds the receiver).
pub fn subscribe() -> broadcast::Receiver<UiEvent> {
    bus().subscribe()
}

/// Emit one event. No-op when nobody listens (cheap: send fails silently).
pub fn emit(event: UiEvent) {
    let _ = bus().send(event);
}

static TAKEOVER: AtomicBool = AtomicBool::new(false);

/// `true` while the fullscreen TUI owns the terminal.
pub fn ui_takeover() -> bool {
    TAKEOVER.load(Ordering::SeqCst)
}

/// Called by the TUI on entry/exit (restores terminal printing on exit).
pub fn set_ui_takeover(active: bool) {
    TAKEOVER.store(active, Ordering::SeqCst);
}

/// Engine println replacement: event when the TUI owns the screen.
pub fn tui_log(line: String) {
    if ui_takeover() {
        emit(UiEvent::Log { line });
    } else {
        println!("{}", line);
    }
}

/// Engine eprintln replacement: event when the TUI owns the screen.
pub fn tui_warn(line: String) {
    if ui_takeover() {
        emit(UiEvent::Warn { line });
    } else {
        eprintln!("{}", line);
    }
}

fn approval_hook() -> &'static std::sync::Mutex<Option<mpsc::UnboundedSender<ApprovalRequest>>> {
    static HOOK: OnceLock<std::sync::Mutex<Option<mpsc::UnboundedSender<ApprovalRequest>>>> =
        OnceLock::new();
    HOOK.get_or_init(|| std::sync::Mutex::new(None))
}

/// Installed by the TUI; cleared on exit. CLI prompts are untouched when empty.
pub fn set_approval_hook(hook: Option<mpsc::UnboundedSender<ApprovalRequest>>) {
    *approval_hook().lock().unwrap() = hook;
}

/// `true` while the TUI can answer permission requests.
pub fn has_approval_hook() -> bool {
    approval_hook().lock().unwrap().is_some()
}

/// Ask for approval: modal in TUI mode, `None` when the CLI owns stdin.
/// A dropped TUI (send/recv failure) fails closed → caller must treat as reject.
pub async fn request_approval(prompt: String, diff: Vec<String>) -> Option<ApprovalChoice> {
    let tx = approval_hook().lock().unwrap().clone()?;
    let (respond, rx) = oneshot::channel();
    tx.send(ApprovalRequest { prompt, diff, respond }).ok()?;
    rx.await.ok()
}
