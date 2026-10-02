//! Command-center TUI: header HUD, anime sidekick, chat, tasks, project,
//! status bar, input with command palette, permission modals, diff viewer.
//!
//! The app owns the screen and consumes [`crate::core::ui_events`]; it never
//! calls the engine except through [`super::bridge`] tasks. Engine output
//! arrives as events (terminal printing reroutes automatically).

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event as CEvent, EventStream, KeyCode, KeyModifiers};
use futures::StreamExt;
use ratatui::{prelude::*, widgets::*};
use tokio::sync::mpsc;

use super::anime::{self, AnimeState};
use super::theme::{self, TermCaps, Theme};
use crate::core::{
    config::ProviderKind,
    fs as core_fs, git as core_git, intelligence,
    projects::Project,
    tools as core_tools,
    ui_events::{self, ApprovalChoice, ApprovalRequest, UiEvent},
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Chat,
    Tasks,
    Project,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Self::Chat => Self::Tasks,
            Self::Tasks => Self::Project,
            Self::Project => Self::Chat,
        }
    }
}

#[derive(Debug, Clone)]
struct ChatLine {
    role: &'static str,
    text: String,
}

#[derive(Debug, Clone)]
struct TaskRow {
    id: String,
    title: String,
    kind: String,
    status: String,
}

#[derive(Debug, Clone)]
struct ToolRow {
    label: String,
    status: String,
}

#[derive(Debug)]
struct PendingApproval {
    prompt: String,
    diff: Vec<String>,
    respond: Option<tokio::sync::oneshot::Sender<ApprovalChoice>>,
}

struct PaletteItem {
    name: &'static str,
    desc: &'static str,
}

const PALETTE: &[PaletteItem] = &[
    PaletteItem { name: "/help", desc: "show commands and keys" },
    PaletteItem { name: "/model", desc: "list models / switch model (/model <id>)" },
    PaletteItem { name: "/theme", desc: "cycle theme (/theme <name>)" },
    PaletteItem { name: "/ascii", desc: "toggle ASCII faces" },
    PaletteItem { name: "/animation", desc: "toggle animation" },
    PaletteItem { name: "/clear", desc: "clear chat" },
    PaletteItem { name: "/quit", desc: "exit the TUI" },
];

pub struct TuiOptions {
    pub project: Option<String>,
    pub no_animation: bool,
    pub ascii: bool,
    pub theme: Option<String>,
}

struct App {
    proj: Project,
    files: Vec<core_fs::ProjectFile>,
    branch: String,
    git_summary: String,
    provider_kind: ProviderKind,
    provider_url: String,
    model: Option<String>,
    model_override: Option<String>,
    theme: Theme,
    theme_idx: usize,
    caps: TermCaps,
    anime: AnimeState,
    frame: usize,
    line_idx: usize,
    last_event: Instant,
    started: Instant,
    messages: Vec<ChatLine>,
    pending_answer: String,
    chat_scroll: u16,
    tasks: Vec<TaskRow>,
    task_sel: usize,
    total_steps: usize,
    tools: Vec<ToolRow>,
    tool_count: usize,
    tests_pass: usize,
    tests_fail: usize,
    input: String,
    history: Vec<String>,
    hist_idx: Option<usize>,
    focus: Focus,
    palette_open: bool,
    palette_sel: usize,
    approval: Option<PendingApproval>,
    running: bool,
    run_goal_text: String,
    run_started: Instant,
    run_handle: Option<tokio::task::JoinHandle<()>>,
    continue_offer: Option<String>,
    should_quit: bool,
    width: u16,
    height: u16,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(opts: TuiOptions) -> Result<()> {
    use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
    use crossterm::ExecutableCommand;

    if !console::user_attended() {
        anyhow::bail!("`tui` needs an interactive terminal — use the plain subcommands in scripts/CI");
    }

    let proj = crate::core::projects::resolve_project(opts.project)?;
    let cfg = crate::core::config::load_config().unwrap_or_default();
    let provider_kind = cfg.provider.active.clone();
    let provider_url =
        crate::core::config::resolve_provider_url(&provider_kind, &cfg, None, None);
    let caps = theme::detect_caps(opts.ascii, opts.no_animation);
    let mut app_theme = match opts.theme.as_deref() {
        Some(name) => theme::builtin(name),
        None => theme::load_custom().unwrap_or_default(),
    };
    if !caps.colors {
        app_theme = theme::builtin("monochrome");
    }

    let mut app = App {
        proj,
        files: Vec::new(),
        branch: String::new(),
        git_summary: String::new(),
        provider_kind,
        provider_url,
        model: None,
        model_override: None,
        theme: app_theme,
        theme_idx: 0,
        caps,
        anime: AnimeState::Idle,
        frame: 0,
        line_idx: 0,
        last_event: Instant::now(),
        started: Instant::now(),
        messages: Vec::new(),
        pending_answer: String::new(),
        chat_scroll: 0,
        tasks: Vec::new(),
        task_sel: 0,
        total_steps: 0,
        tools: Vec::new(),
        tool_count: core_tools::project_tools().len(),
        tests_pass: 0,
        tests_fail: 0,
        input: String::new(),
        history: Vec::new(),
        hist_idx: None,
        focus: Focus::Chat,
        palette_open: false,
        palette_sel: 0,
        approval: None,
        running: false,
        run_goal_text: String::new(),
        run_started: Instant::now(),
        run_handle: None,
        continue_offer: None,
        should_quit: false,
        width: 0,
        height: 0,
    };

    terminal::enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    ui_events::set_ui_takeover(true);
    let (approval_tx, mut approval_rx) = mpsc::unbounded_channel::<ApprovalRequest>();
    ui_events::set_approval_hook(Some(approval_tx));
    let mut bus_rx = ui_events::subscribe();

    if !opts.no_animation {
        startup_animation(&mut terminal, &app).await?;
    }
    startup_checks(&mut app).await;
    greet(&mut app);

    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    let mut events = EventStream::new();
    let mut tick: u64 = 0;

    loop {
        tokio::select! {
            maybe_ev = events.next() => {
                match maybe_ev {
                    Some(Ok(ev)) => handle_terminal_event(&mut app, ev).await?,
                    Some(Err(_)) => break,
                    None => break,
                }
            }
            bus_msg = bus_rx.recv() => {
                match bus_msg {
                    Ok(ev) => apply_event(&mut app, ev),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            req = approval_rx.recv() => {
                if let Some(req) = req {
                    open_approval(&mut app, req);
                }
            }
            _ = ticker.tick() => {
                tick += 1;
                on_tick(&mut app, tick);
            }
        }
        if app.should_quit {
            break;
        }
        terminal.draw(|f| render(f, &mut app))?;
    }

    if let Some(h) = app.run_handle.take() {
        h.abort();
    }
    ui_events::set_approval_hook(None);
    ui_events::set_ui_takeover(false);
    terminal::disable_raw_mode()?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    println!("bye — session traces are in ~/.cache/local-ai/");
    Ok(())
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

async fn startup_animation(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &App,
) -> Result<()> {
    let frames = [
        ["      ▄█▀█▄     ", "     █ ◉ ◉ █     ", "      █ ▿ █      ", "                ", "    LOCAL-AI    "],
        ["      ▄█▀█▄     ", "     █ ◉ ◉ █ ▸   ", "      █ ▿ █      ", "                ", "    LOCAL-AI    "],
        ["      ▄█▀█▄     ", "   ◂ █ ◉ ◉ █     ", "      █ ▿ █      ", "                ", "    LOCAL-AI    "],
    ];
    for frame in frames {
        terminal.draw(|f| {
            let area = f.area();
            let lines: Vec<Line> = frame
                .iter()
                .map(|l| Line::from(Span::styled(*l, Style::default().fg(app.theme.character))))
                .collect();
            let h = lines.len() as u16;
            let y = area.height.saturating_sub(h) / 2;
            let w = frame.iter().map(|l| l.chars().count() as u16).max().unwrap_or(0);
            let x = area.width.saturating_sub(w) / 2;
            f.render_widget(
                Paragraph::new(lines),
                Rect::new(x.min(area.width), y.min(area.height), w.min(area.width), h.min(area.height)),
            );
        })?;
        tokio::time::sleep(Duration::from_millis(280)).await;
    }
    Ok(())
}

async fn startup_checks(app: &mut App) {
    // Model.
    let cfg = crate::core::config::load_config().unwrap_or_default();
    match crate::core::provider::list_models_unified(&app.provider_kind, &app.provider_url, &cfg).await
    {
        Ok(models) if !models.is_empty() => {
            app.model = Some(models[0].id.clone());
            push_sys(
                app,
                format!("✓ Model connected ({} via {})", models[0].id, app.provider_kind),
            );
        }
        _ => push_sys(app, "○ No model reachable — Q&A and coder steps will wait for one".into()),
    }
    // Project.
    app.files = core_fs::list_project_files(&app.proj).unwrap_or_default();
    let n_files = app.files.iter().filter(|f| !f.is_directory).count();
    push_sys(app, format!("✓ Project loaded ({} — {} files)", app.proj.name, n_files));
    refresh_git(app);
    push_sys(app, format!("✓ Tools ready ({})", app.tool_count));
    // Pending mission → continue offer (§16 session greeting).
    if let Ok(missions) = crate::core::missions::read_missions() {
        if let Some(m) = missions
            .iter()
            .filter(|m| m.project_id == app.proj.id && !m.status.is_terminal())
            .max_by_key(|m| m.number)
        {
            app.continue_offer = Some(m.goal.clone());
        }
    }
}

fn greet(app: &mut App) {
    let hour = chrono::Local::now().format("%H").to_string().parse::<u32>().unwrap_or(12);
    let part = if hour < 12 { "morning" } else if hour < 18 { "afternoon" } else { "evening" };
    let user = std::env::var("USER").unwrap_or_else(|_| "dev".into());
    push_ai(app, format!("Good {}, {}. Project: {}", part, user, app.proj.name));
    if let Some(goal) = app.continue_offer.clone() {
        push_ai(
            app,
            format!("Last mission: \"{}\" — type /continue to re-run it, or just send a new goal.", goal),
        );
    } else {
        push_ai(app, "Type a goal and press Enter — approvals stay with you.".into());
    }
    app.anime = AnimeState::Idle;
}

fn refresh_git(app: &mut App) {
    if let Some(folder) = app.proj.folder_path.as_ref() {
        let root = std::path::PathBuf::from(folder);
        if let Ok(root) = root.canonicalize() {
            if let Ok(st) = core_git::status(&root) {
                app.branch = st.branch.clone();
                let dirty = st.staged.len() + st.unstaged.len() + st.untracked.len();
                app.git_summary = if dirty == 0 {
                    "clean".to_string()
                } else {
                    format!("{} change(s)", dirty)
                };
                return;
            }
        }
    }
    app.branch = "—".to_string();
    app.git_summary = "no git repo".to_string();
}

// ---------------------------------------------------------------------------
// Event → state
// ---------------------------------------------------------------------------

fn push_sys(app: &mut App, text: String) {
    app.messages.push(ChatLine { role: "sys", text });
}

fn push_ai(app: &mut App, text: String) {
    app.messages.push(ChatLine { role: "ai", text });
}

fn touch(app: &mut App) {
    app.last_event = Instant::now();
    if app.anime == AnimeState::Sleeping {
        app.anime = AnimeState::Idle;
    }
}

fn apply_event(app: &mut App, ev: UiEvent) {
    touch(app);
    if let Some(state) = anime::state_for_event(&ev) {
        // Modal + running states win over transient chunk noise.
        if app.approval.is_none() {
            app.anime = state;
            app.frame = 0;
        }
    }
    match ev {
        UiEvent::Log { line } => {
            app.messages.push(ChatLine { role: "log", text: strip_ansi(&line) });
        }
        UiEvent::Warn { line } => {
            app.messages.push(ChatLine { role: "warn", text: strip_ansi(&line) });
        }
        UiEvent::AgentStart { goal, steps, .. } => {
            app.total_steps = steps;
            app.tasks.clear();
            app.task_sel = 0;
            app.tests_pass = 0;
            app.tests_fail = 0;
            push_sys(app, format!("▶ run started: {} ({} steps)", goal, steps));
        }
        UiEvent::PlanCreated { steps } => {
            app.tasks = steps
                .into_iter()
                .map(|(id, title, kind)| TaskRow { id, title, kind, status: "pending".into() })
                .collect();
        }
        UiEvent::StepStart { id, .. } => {
            for t in app.tasks.iter_mut() {
                if t.id == id {
                    t.status = "active".into();
                }
            }
        }
        UiEvent::StepDone { id, status } => {
            for t in app.tasks.iter_mut() {
                if t.id == id {
                    t.status = status.clone();
                }
            }
        }
        UiEvent::ToolActivity { agent, action, detail, status } => {
            app.tools.push(ToolRow {
                label: format!("{} {}", agent, action),
                status: format!("{} — {}", status, detail.chars().take(60).collect::<String>()),
            });
            if app.tools.len() > 50 {
                app.tools.remove(0);
            }
        }
        UiEvent::ModelChunk { text } => {
            app.pending_answer.push_str(&text);
        }
        UiEvent::ModelDone { full } => {
            let text = if full.trim().is_empty() { app.pending_answer.clone() } else { full };
            if !text.trim().is_empty() {
                app.messages.push(ChatLine { role: "ai", text });
            }
            app.pending_answer.clear();
        }
        UiEvent::AnswerStart { .. } => {
            app.pending_answer.clear();
        }
        UiEvent::TestResult { step, passed, summary } => {
            if passed {
                app.tests_pass += 1;
            } else {
                app.tests_fail += 1;
            }
            app.messages.push(ChatLine {
                role: "test",
                text: format!("{} {} — {}", if passed { "✓" } else { "✗" }, step, summary),
            });
        }
        UiEvent::EditProposed { step, files, .. } => {
            app.messages.push(ChatLine {
                role: "tool",
                text: format!("proposed {} edit(s) for {}: {}", files.len(), step, files.join(", ")),
            });
        }
        UiEvent::EditApplied { step, applied } => {
            app.messages.push(ChatLine {
                role: "tool",
                text: format!("applied for {}: {}", step, applied.join(", ")),
            });
            refresh_git(app);
        }
        UiEvent::Review { passed, summary } => {
            app.messages.push(ChatLine {
                role: "review",
                text: format!("review {} — {}", if passed { "pass" } else { "flagged" }, summary),
            });
        }
        UiEvent::AgentComplete { green, summary } => {
            app.running = false;
            app.run_handle = None;
            push_sys(app, format!("{} run finished — {}", if green { "✓ green" } else { "→ done" }, summary));
            refresh_git(app);
        }
    }
}

fn open_approval(app: &mut App, req: ApprovalRequest) {
    app.approval = Some(PendingApproval {
        prompt: req.prompt,
        diff: req.diff,
        respond: Some(req.respond),
    });
    app.anime = AnimeState::WaitingPermission;
    app.frame = 0;
    touch(app);
}

fn answer_approval(app: &mut App, choice: ApprovalChoice) {
    if let Some(mut pending) = app.approval.take() {
        let label = match choice {
            ApprovalChoice::Once => "allowed once",
            ApprovalChoice::Session => "allowed for session",
            ApprovalChoice::Reject => "rejected",
        };
        if let Some(tx) = pending.respond.take() {
            let _ = tx.send(choice);
        }
        app.messages.push(ChatLine { role: "tool", text: format!("permission: {} → {}", pending.prompt, label) });
    }
    app.anime = AnimeState::Idle;
}

fn strip_ansi(line: &str) -> String {
    console::strip_ansi_codes(line).to_string()
}

// ---------------------------------------------------------------------------
// Input: goals, palette, keys
// ---------------------------------------------------------------------------

async fn handle_terminal_event(app: &mut App, ev: CEvent) -> Result<()> {
    let CEvent::Key(key) = ev else { return Ok(()) };
    touch(app);

    // Permission modal owns the keyboard.
    if app.approval.is_some() {
        match key.code {
            KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('1') => {
                answer_approval(app, ApprovalChoice::Once)
            }
            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Char('2') => {
                answer_approval(app, ApprovalChoice::Session)
            }
            KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::Char('3') | KeyCode::Esc => {
                answer_approval(app, ApprovalChoice::Reject)
            }
            _ => {}
        }
        return Ok(());
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            if app.running {
                cancel_run(app);
            } else {
                app.should_quit = true;
            }
        }
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => app.should_quit = true,
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            app.palette_open = !app.palette_open;
            app.palette_sel = 0;
        }
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
            app.messages.clear();
            app.chat_scroll = 0;
        }
        (KeyCode::Tab, _) => {
            app.focus = app.focus.next();
        }
        (KeyCode::Esc, _) => {
            app.palette_open = false;
        }
        (KeyCode::PageUp, _) => {
            app.chat_scroll = app.chat_scroll.saturating_add(10);
        }
        (KeyCode::PageDown, _) => {
            app.chat_scroll = app.chat_scroll.saturating_sub(10);
        }
        (KeyCode::Up, _) if app.palette_open => {
            app.palette_sel = app.palette_sel.saturating_sub(1);
        }
        (KeyCode::Down, _) if app.palette_open => {
            app.palette_sel = app.palette_sel.saturating_add(1);
        }
        (KeyCode::Up, _) => {
            if app.focus == Focus::Tasks {
                app.task_sel = app.task_sel.saturating_sub(1);
            } else if let Some(prev) = history_prev(app) {
                app.input = prev;
            }
        }
        (KeyCode::Down, _) => {
            if app.focus == Focus::Tasks {
                app.task_sel = app.task_sel.saturating_add(1);
            } else if let Some(next) = history_next(app) {
                app.input = next;
            }
        }
        (KeyCode::Enter, _) => {
            if app.palette_open {
                run_palette_selection(app).await?;
            } else {
                submit_input(app).await;
            }
        }
        (KeyCode::Backspace, _) => {
            app.input.pop();
            app.hist_idx = None;
            sync_palette_trigger(app);
        }
        (KeyCode::Char(ch), _) => {
            app.input.push(ch);
            app.hist_idx = None;
            sync_palette_trigger(app);
        }
        _ => {}
    }
    Ok(())
}

/// Typing `/` opens the command palette; clearing it closes it.
fn sync_palette_trigger(app: &mut App) {
    if app.input.starts_with('/') && !app.palette_open {
        app.palette_open = true;
        app.palette_sel = 0;
    } else if !app.input.starts_with('/') && app.palette_open {
        app.palette_open = false;
    }
}

fn history_prev(app: &mut App) -> Option<String> {
    if app.history.is_empty() {
        return None;
    }
    let next = match app.hist_idx {
        None => app.history.len().saturating_sub(1),
        Some(i) => i.saturating_sub(1),
    };
    app.hist_idx = Some(next);
    app.history.get(next).cloned()
}

fn history_next(app: &mut App) -> Option<String> {
    let i = app.hist_idx?;
    if i + 1 >= app.history.len() {
        app.hist_idx = None;
        return Some(String::new());
    }
    app.hist_idx = Some(i + 1);
    app.history.get(i + 1).cloned()
}

fn cancel_run(app: &mut App) {
    if let Some(h) = app.run_handle.take() {
        h.abort();
    }
    app.running = false;
    app.messages.push(ChatLine { role: "warn", text: "run cancelled (Ctrl+C)".into() });
    app.anime = AnimeState::Idle;
}

async fn submit_input(app: &mut App) {
    let text = app.input.trim().to_string();
    if text.is_empty() {
        return;
    }
    app.input.clear();
    app.history.push(text.clone());
    app.hist_idx = None;
    if app.running {
        app.messages.push(ChatLine { role: "warn", text: "a run is already active (Ctrl+C to cancel it)".into() });
        return;
    }
    if text == "/continue" {
        if let Some(goal) = app.continue_offer.clone() {
            start_goal(app, goal).await;
        } else {
            app.messages.push(ChatLine { role: "warn", text: "no pending mission to continue".into() });
        }
        return;
    }
    if let Some(rest) = text.strip_prefix('/') {
        run_slash(app, rest).await;
        return;
    }
    start_goal(app, text).await;
}

/// Route by intent: action goals run the plan loop, questions get answers.
async fn start_goal(app: &mut App, goal: String) {
    use intelligence::ProjectIntent;
    app.messages.push(ChatLine { role: "user", text: goal.clone() });
    app.run_goal_text = goal.clone();
    app.running = true;
    app.run_started = Instant::now();
    app.anime = AnimeState::Excited;
    let intent = intelligence::detect_intent(&goal);
    let proj = app.proj.clone();
    let kind = app.provider_kind.clone();
    let url = app.provider_url.clone();
    let model = app.model_override.clone();
    match intent.intent {
        ProjectIntent::Edit | ProjectIntent::Debug | ProjectIntent::Review | ProjectIntent::Run => {
            let handle = tokio::spawn(async move {
                match super::bridge::run_goal(&proj, &goal, &kind, &url, model).await {
                    Ok(summary) => ui_events::emit(UiEvent::Log { line: format!("run report: {}", summary) }),
                    Err(e) => {
                        ui_events::emit(UiEvent::Warn { line: format!("run failed: {}", e) });
                        ui_events::emit(UiEvent::AgentComplete { green: false, summary: e.to_string() });
                    }
                }
            });
            app.run_handle = Some(handle);
        }
        _ => {
            let handle = tokio::spawn(async move {
                match super::bridge::answer_once(&proj, &goal, &kind, &url, model).await {
                    Ok(_) => {}
                    Err(e) => {
                        ui_events::emit(UiEvent::Warn { line: format!("answer failed: {}", e) });
                    }
                }
            });
            app.run_handle = Some(handle);
        }
    }
}

fn palette_items(app: &App) -> Vec<&'static PaletteItem> {
    let q = app.input.trim_start_matches('/').to_lowercase();
    PALETTE
        .iter()
        .filter(|p| q.is_empty() || p.name.contains(&q) || p.desc.to_lowercase().contains(&q))
        .collect()
}

async fn run_palette_selection(app: &mut App) -> Result<()> {
    let items = palette_items(app);
    let sel = items.get(app.palette_sel.min(items.len().saturating_sub(1))).map(|p| p.name);
    app.palette_open = false;
    app.input.clear();
    match sel {
        Some("/quit") => app.should_quit = true,
        Some("/clear") => {
            app.messages.clear();
            app.chat_scroll = 0;
        }
        Some("/ascii") => {
            app.caps.unicode = !app.caps.unicode;
            push_sys(app, format!("ascii faces {}", if app.caps.unicode { "off" } else { "on" }));
        }
        Some("/animation") => {
            app.caps.animation = !app.caps.animation;
            push_sys(app, format!("animation {}", if app.caps.animation { "on" } else { "off" }));
        }
        Some("/theme") => cycle_theme(app),
        Some("/model") => list_models(app).await,
        Some("/help") => print_palette_help(app),
        _ => {}
    }
    Ok(())
}

async fn run_slash(app: &mut App, rest: &str) {
    let mut parts = rest.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    match cmd {
        "quit" | "exit" => app.should_quit = true,
        "clear" => {
            app.messages.clear();
            app.chat_scroll = 0;
        }
        "help" => print_palette_help(app),
        "ascii" => {
            app.caps.unicode = !app.caps.unicode;
            push_sys(app, format!("ascii faces {}", if app.caps.unicode { "off" } else { "on" }));
        }
        "animation" => {
            app.caps.animation = !app.caps.animation;
            push_sys(app, format!("animation {}", if app.caps.animation { "on" } else { "off" }));
        }
        "theme" => match parts.next() {
            Some(name) => set_theme(app, name),
            None => cycle_theme(app),
        },
        "model" => match parts.next() {
            Some(id) => {
                app.model_override = Some(id.to_string());
                push_sys(app, format!("model override → {}", id));
            }
            None => list_models(app).await,
        },
        "continue" => {
            if let Some(goal) = app.continue_offer.clone() {
                start_goal(app, goal).await;
            } else {
                push_sys(app, "no pending mission to continue".into());
            }
        }
        _ => push_sys(app, format!("unknown command /{} — try /help", cmd)),
    }
}

fn print_palette_help(app: &mut App) {
    push_sys(
        app,
        "commands: /help /model [/model <id>] /theme [/theme <name>] /ascii /animation /clear /quit — \
         keys: Enter send · ↑↓ history/tasks · Tab panels · PgUp/PgDn scroll · Ctrl+P palette · Ctrl+C cancel · Ctrl+D quit"
            .into(),
    );
}

async fn list_models(app: &mut App) {
    let cfg = crate::core::config::load_config().unwrap_or_default();
    match crate::core::provider::list_models_unified(&app.provider_kind, &app.provider_url, &cfg).await
    {
        Ok(models) if !models.is_empty() => {
            let ids: Vec<String> = models.iter().take(12).map(|m| m.id.clone()).collect();
            push_sys(app, format!("models ({}): {} — switch with /model <id>", models.len(), ids.join(", ")));
        }
        _ => push_sys(app, "no models reachable — start Ollama or LM Studio".into()),
    }
}

fn cycle_theme(app: &mut App) {
    let names = theme::builtin_names();
    app.theme_idx = (app.theme_idx + 1) % names.len();
    set_theme(app, names[app.theme_idx]);
}

fn set_theme(app: &mut App, name: &str) {
    if name.eq_ignore_ascii_case("custom") {
        if let Some(custom) = theme::load_custom() {
            app.theme = custom;
            push_sys(app, "theme → custom (~/.config/local-ai/theme.json)".into());
            return;
        }
        push_sys(app, "no custom theme.json — keeping current theme".into());
        return;
    }
    app.theme = theme::builtin(name);
    if !app.caps.colors {
        app.theme = theme::builtin("monochrome");
    }
    push_sys(app, format!("theme → {}", app.theme.name));
}

fn on_tick(app: &mut App, tick: u64) {
    // Animation frames + slow personality-line rotation.
    if app.caps.animation && tick.is_multiple_of(3) {
        let n = anime::frames(app.anime).len();
        if n > 1 {
            app.frame = (app.frame + 1) % n;
        }
    }
    if tick.is_multiple_of(100) {
        let n = anime::lines(app.anime).len();
        if n > 1 {
            app.line_idx = (app.line_idx + 1) % n;
        }
    }
    // Sleep after 90s of silence (idle runs only).
    if !app.running && app.approval.is_none() && app.last_event.elapsed() > Duration::from_secs(90) {
        app.anime = AnimeState::Sleeping;
    }
    // Git panel refresh every 10s.
    if tick.is_multiple_of(100) {
        refresh_git(app);
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn task_icon(status: &str) -> (&'static str, bool) {
    match status {
        "done" | "pass" => ("✓", true),
        "active" => ("→", true),
        "fail" | "blocked" | "refused" | "flagged" => ("✗", false),
        _ => ("○", false),
    }
}

fn render(f: &mut Frame, app: &mut App) {
    let size = f.area();
    app.width = size.width;
    app.height = size.height;

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(size);
    render_header(f, app, rows[0]);

    if size.width >= 120 {
        let cols = Layout::horizontal([Constraint::Length(24), Constraint::Min(0)]).split(rows[1]);
        render_anime(f, app, cols[0]);
        let right =
            Layout::vertical([Constraint::Min(0), Constraint::Length(11)]).split(cols[1]);
        render_chat(f, app, right[0]);
        let bottom = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(right[1]);
        render_tasks(f, app, bottom[0]);
        render_project(f, app, bottom[1]);
    } else if size.width >= 80 {
        let cols = Layout::horizontal([Constraint::Length(18), Constraint::Min(0)]).split(rows[1]);
        render_anime(f, app, cols[0]);
        let right =
            Layout::vertical([Constraint::Min(0), Constraint::Length(8)]).split(cols[1]);
        render_chat(f, app, right[0]);
        render_tasks(f, app, right[1]);
    } else {
        render_chat(f, app, rows[1]);
    }

    render_status(f, app, rows[2]);
    render_input(f, app, rows[3]);

    if app.palette_open {
        render_palette(f, app, size);
    }
    if app.approval.is_some() {
        render_approval(f, app, size);
    }
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let model = app.model.as_deref().unwrap_or("no model");
    let done = app.tasks.iter().filter(|t| t.status != "pending" && t.status != "active").count();
    let pct = if app.total_steps == 0 { 0 } else { done * 100 / app.total_steps.max(1) };
    let line = Line::from(vec![
        Span::styled(" LOCAL-AI ", Style::default().fg(app.theme.header).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" {} │ 128K │ ● LOCAL │ {} │ {}% ", model, app.branch, pct),
            Style::default().fg(app.theme.muted),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_anime(f: &mut Frame, app: &App, area: Rect) {
    let frames = anime::frames(app.anime);
    let frame = frames[app.frame % frames.len()];
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("{:?}", app.anime).to_uppercase().replace("WAITINGPERMISSION", "WAITING"),
        Style::default().fg(app.theme.muted),
    ))];
    for row in frame.iter() {
        let row = if app.caps.unicode { row.to_string() } else { anime::ascii_line(row) };
        lines.push(Line::from(Span::styled(row, Style::default().fg(app.theme.character))));
    }
    let quips = anime::lines(app.anime);
    lines.push(Line::from(Span::styled(
        format!("\"{}\"", quips[app.line_idx % quips.len()]),
        Style::default().fg(app.theme.muted).add_modifier(Modifier::ITALIC),
    )));
    let block = Block::bordered()
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(" AI ", Style::default().fg(app.theme.accent)));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn chat_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let wrap = width.saturating_sub(4).max(20);
    let push_wrapped = |out: &mut Vec<Line>, prefix: String, text: &str, color: Color| {
        out.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]));
        for chunk in textwrap_chunks(text, wrap) {
            out.push(Line::from(Span::styled(format!("  {}", chunk), Style::default().fg(color))));
        }
    };
    for m in &app.messages {
        match m.role {
            "user" => push_wrapped(&mut out, "YOU".into(), &m.text, app.theme.accent),
            "ai" => push_wrapped(&mut out, "LOCAL-AI".into(), &m.text, Color::White),
            "sys" => push_wrapped(&mut out, "·".into(), &m.text, app.theme.muted),
            "warn" => push_wrapped(&mut out, "!".into(), &m.text, app.theme.warning),
            "test" => push_wrapped(&mut out, "TEST".into(), &m.text, app.theme.accent),
            "tool" => push_wrapped(&mut out, "●".into(), &m.text, app.theme.muted),
            "review" => push_wrapped(&mut out, "REVIEW".into(), &m.text, app.theme.warning),
            "log" => push_wrapped(&mut out, "·".into(), &m.text, app.theme.muted),
            _ => push_wrapped(&mut out, "·".into(), &m.text, app.theme.muted),
        }
    }
    if !app.pending_answer.is_empty() {
        push_wrapped(&mut out, "LOCAL-AI…".into(), &app.pending_answer, Color::White);
    }
    out
}

/// Greedy word wrap (no extra deps).
fn textwrap_chunks(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > width {
                out.push(line);
                line = String::new();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            // Hard-split pathological long tokens (hashes, URLs).
            if word.len() > width {
                if !line.is_empty() {
                    out.push(line);
                }
                let mut rest = word;
                while rest.len() > width {
                    let cut = rest
                        .char_indices()
                        .take(width)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(width);
                    out.push(rest[..cut].to_string());
                    rest = &rest[cut..];
                }
                line = rest.to_string();
            } else {
                line.push_str(word);
            }
        }
        out.push(line);
    }
    out
}

fn render_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let lines = chat_lines(app, area.width as usize);
    let total = lines.len() as u16;
    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = total.saturating_sub(visible as u16);
    app.chat_scroll = app.chat_scroll.min(max_scroll);
    let scroll = max_scroll.saturating_sub(app.chat_scroll);
    let title = if app.focus == Focus::Chat { " CHAT ● " } else { " CHAT " };
    let block = Block::bordered()
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(title, Style::default().fg(app.theme.accent)));
    f.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
}

fn render_tasks(f: &mut Frame, app: &mut App, area: Rect) {
    let mut items = Vec::new();
    if app.tasks.is_empty() {
        items.push(Line::from(Span::styled("  no plan yet — send a goal", Style::default().fg(app.theme.muted))));
    }
    for (i, t) in app.tasks.iter().enumerate() {
        let (icon, good) = task_icon(&t.status);
        let color = if good { app.theme.success } else if t.status == "active" { app.theme.accent } else { app.theme.muted };
        let marker = if app.focus == Focus::Tasks && i == app.task_sel { "▸" } else { " " };
        items.push(Line::from(vec![
            Span::styled(format!("{} {} ", marker, icon), Style::default().fg(color)),
            Span::styled(t.title.clone(), Style::default().fg(Color::White)),
            Span::styled(format!(" [{}]", t.kind), Style::default().fg(app.theme.muted)),
        ]));
    }
    let title = if app.focus == Focus::Tasks { " PLAN ● " } else { " PLAN " };
    let block = Block::bordered()
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(title, Style::default().fg(app.theme.accent)));
    f.render_widget(Paragraph::new(items).block(block), area);
}

fn render_project(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = vec![Line::from(vec![
        Span::styled(app.proj.name.clone(), Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("  {} · {}", app.branch, app.git_summary),
            Style::default().fg(app.theme.muted),
        ),
    ])];
    // Shallow tree: top-level dirs + loose files, capped to the panel.
    let mut shown = 0;
    let mut top: Vec<String> = app
        .files
        .iter()
        .filter(|f| !f.is_directory)
        .map(|f| f.path.split('/').next().unwrap_or(&f.path).to_string())
        .collect();
    top.sort();
    top.dedup();
    for entry in top.iter().take((area.height as usize).saturating_sub(4).max(1)) {
        lines.push(Line::from(Span::styled(format!("  ├── {}", entry), Style::default().fg(Color::White))));
        shown += 1;
    }
    if top.len() > shown {
        lines.push(Line::from(Span::styled(
            format!("  └── … ({} more)", top.len() - shown),
            Style::default().fg(app.theme.muted),
        )));
    }
    let title = if app.focus == Focus::Project { " PROJECT ● " } else { " PROJECT " };
    let block = Block::bordered()
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(title, Style::default().fg(app.theme.accent)));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let state = format!("{:?}", app.anime).to_lowercase();
    let elapsed = app.run_started.elapsed().as_secs();
    let elapsed_str = if app.running {
        format!("{}s", elapsed)
    } else {
        format!("{}s idle", app.started.elapsed().as_secs())
    };
    let done = app.tasks.iter().filter(|t| t.status != "pending" && t.status != "active").count();
    let last_tool = app.tools.last().map(|t| format!("{}: {}", t.label, t.status)).unwrap_or_default();
    let line = Line::from(vec![
        Span::styled(
            format!(" ● {} ", state),
            Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("│ {} tools │ ✓{} ✗{} │ {} │ {}/{} steps │ {} ",
                app.tool_count, app.tests_pass, app.tests_fail, elapsed_str, done, app.total_steps, app.branch),
            Style::default().fg(app.theme.muted),
        ),
        Span::styled(last_tool.chars().take(48).collect::<String>(), Style::default().fg(app.theme.muted)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let hint = if app.input.is_empty() { " Type a goal, / for commands…" } else { "" };
    // ASCII prompt: guaranteed single-cell columns in every terminal.
    let text = format!("> {}{}", app.input, hint);
    let block = Block::bordered()
        .border_style(Style::default().fg(app.theme.border))
        .title(Span::styled(" INPUT ", Style::default().fg(app.theme.accent)));
    f.render_widget(Paragraph::new(Line::from(Span::styled(text, Style::default().fg(Color::White)))).block(block), area);
    // Cursor sits past the prompt + input using display widths, not char counts.
    let cx = area.x.saturating_add(input_cursor_col(&app.input));
    let cy = area.y + 1;
    if cx < area.x + area.width && cy < area.y + area.height {
        f.set_cursor_position(ratatui::layout::Position::new(cx, cy));
    }
}

/// Display column of the cursor relative to the input box origin:
/// border(1) + prompt("> ", 2) + display width of the typed text.
fn input_cursor_col(input: &str) -> u16 {
    use unicode_width::UnicodeWidthStr;
    1 + 2 + UnicodeWidthStr::width(input) as u16
}

fn render_palette(f: &mut Frame, app: &mut App, area: Rect) {
    let items = palette_items(app);
    let w = 44.min(area.width.saturating_sub(4)).max(30);
    let h = (items.len() as u16 + 4).min(area.height.saturating_sub(4)).max(6);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(h) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == app.palette_sel.min(items.len().saturating_sub(1)) {
                Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<10} ", p.name), style),
                Span::styled(p.desc, Style::default().fg(app.theme.muted)),
            ]))
        })
        .collect();
    let title = format!(" COMMANDS ({}) ", app.input);
    f.render_widget(
        List::new(list_items).block(
            Block::bordered()
                .border_style(Style::default().fg(app.theme.accent))
                .title(Span::styled(title, Style::default().fg(app.theme.accent))),
        ),
        popup,
    );
}

fn render_approval(f: &mut Frame, app: &App, area: Rect) {
    let Some(pending) = app.approval.as_ref() else { return };
    let w = 76.min(area.width.saturating_sub(4)).max(40);
    let shown: Vec<String> = pending.diff.iter().take(22).cloned().collect();
    let h = (shown.len() as u16 + 9).min(area.height.saturating_sub(2)).max(10);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(h) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);
    let mut lines = vec![
        Line::from(Span::styled(
            "⚠ PERMISSION REQUIRED",
            Style::default().fg(app.theme.warning).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(pending.prompt.clone(), Style::default().fg(Color::White))),
        Line::from(Span::styled("─ diff ─".to_string(), Style::default().fg(app.theme.muted))),
    ];
    for d in shown {
        let (prefix, color) = if let Some(rest) = d.strip_prefix('+') {
            (format!("+{}", rest), app.theme.success)
        } else if let Some(rest) = d.strip_prefix('-') {
            (format!("-{}", rest), app.theme.error)
        } else if d.starts_with('!') {
            (d.clone(), app.theme.warning)
        } else {
            (d.clone(), app.theme.muted)
        };
        lines.push(Line::from(Span::styled(prefix, Style::default().fg(color))));
    }
    if pending.diff.len() > 22 {
        lines.push(Line::from(Span::styled(
            format!("… ({} more lines)", pending.diff.len() - 22),
            Style::default().fg(app.theme.muted),
        )));
    }
    lines.push(Line::from(vec![
        Span::styled("[a] Allow once   ", Style::default().fg(app.theme.success).add_modifier(Modifier::BOLD)),
        Span::styled("[s] Allow session   ", Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled("[r] Reject", Style::default().fg(app.theme.error).add_modifier(Modifier::BOLD)),
    ]));
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(Style::default().fg(app.theme.warning))
                .title(Span::styled(" APPROVAL ", Style::default().fg(app.theme.warning))),
        ),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn sample_app() -> App {
        App {
            proj: Project {
                id: "test".into(),
                name: "demo".into(),
                created_at: String::new(),
                updated_at: String::new(),
                folder_path: None,
                messages: Vec::new(),
            },
            files: Vec::new(),
            branch: "main".into(),
            git_summary: "clean".into(),
            provider_kind: ProviderKind::Auto,
            provider_url: String::new(),
            model: Some("qwen-test".into()),
            model_override: None,
            theme: Theme::default(),
            theme_idx: 0,
            caps: TermCaps { colors: true, unicode: true, animation: false },
            anime: AnimeState::Thinking,
            frame: 0,
            line_idx: 0,
            last_event: Instant::now(),
            started: Instant::now(),
            messages: vec![
                ChatLine { role: "user", text: "fix it".into() },
                ChatLine { role: "ai", text: "on it".into() },
            ],
            pending_answer: String::new(),
            chat_scroll: 0,
            tasks: vec![TaskRow {
                id: "edit-1".into(),
                title: "Modify src/a.rs".into(),
                kind: "edit".into(),
                status: "active".into(),
            }],
            task_sel: 0,
            total_steps: 5,
            tools: vec![ToolRow { label: "tester test".into(), status: "pass".into() }],
            tool_count: 7,
            tests_pass: 3,
            tests_fail: 0,
            input: String::new(),
            history: Vec::new(),
            hist_idx: None,
            focus: Focus::Chat,
            palette_open: false,
            palette_sel: 0,
            approval: None,
            running: true,
            run_goal_text: "fix it".into(),
            run_started: Instant::now(),
            run_handle: None,
            continue_offer: None,
            should_quit: false,
            width: 0,
            height: 0,
        }
    }

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn render_all_sizes_without_panic() {
        // Full, medium, small, and tiny viewports.
        for (w, h) in [(140u16, 40u16), (100, 30), (60, 20), (50, 15)] {
            let backend = TestBackend::new(w, h);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut app = sample_app();
            terminal.draw(|f| render(f, &mut app)).unwrap();
            let text = screen_text(&terminal);
            assert!(text.contains("LOCAL-AI"), "{}x{}", w, h);
            assert!(text.contains("CHAT"), "{}x{}", w, h);
        }
    }

    #[test]
    fn render_palette_and_approval_without_panic() {
        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = sample_app();
        app.input = "/th".into();
        app.palette_open = true;
        terminal.draw(|f| render(f, &mut app)).unwrap();
        assert!(screen_text(&terminal).contains("COMMANDS"));

        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.palette_open = false;
        app.approval = Some(PendingApproval {
            prompt: "Apply 1 edit?".into(),
            diff: vec!["-old".into(), "+new".into()],
            respond: Some(tx),
        });
        // Hold the receiver alive for the draw.
        let _keep = _rx;
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = screen_text(&terminal);
        assert!(text.contains("PERMISSION"), "{}", &text[..text.len().min(200)]);
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::input_cursor_col;

    #[test]
    fn cursor_accounts_for_display_width() {
        // border(1) + "> "(2) + text width.
        assert_eq!(input_cursor_col(""), 3);
        assert_eq!(input_cursor_col("abc"), 6);
        // CJK is double-cell: chars().count() would say 5, display says 7.
        assert_eq!(input_cursor_col("日本"), 7);
        assert_eq!(input_cursor_col("a日本b"), 9);
    }
}
