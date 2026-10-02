//! Anime character engine — a state-driven visual component, not decoration.
//!
//! The character mirrors [`crate::core::ui_events::UiEvent`]s: every agent
//! event maps to a state, each state has face frames + a rotating
//! personality line. Animated states cycle 2–4 frames on the render tick;
//! everything else is a single held frame. On terminals without Unicode
//! (`--ascii` or no UTF-8 locale) faces degrade through [`ascii_face`].

use crate::core::ui_events::UiEvent;

/// Visual states. The TUI maps agent events to these via [`state_for_event`];
/// `Sleeping` is entered by the app after an idle timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimeState {
    #[default]
    Idle,
    Listening,
    Thinking,
    Planning,
    Searching,
    Reading,
    Coding,
    Editing,
    Running,
    WaitingPermission,
    Success,
    Warning,
    Error,
    Confused,
    Curious,
    Excited,
    Sleeping,
}

/// Map an engine event to the character state it implies.
pub fn state_for_event(event: &UiEvent) -> Option<AnimeState> {
    match event {
        UiEvent::Log { .. } | UiEvent::Warn { .. } => None,
        UiEvent::AgentStart { .. } => Some(AnimeState::Excited),
        UiEvent::PlanCreated { .. } => Some(AnimeState::Planning),
        UiEvent::StepStart { kind, .. } => Some(match kind.as_str() {
            "inspect" => AnimeState::Searching,
            "context" => AnimeState::Reading,
            "test" => AnimeState::Running,
            "edit" | "create" => AnimeState::Coding,
            "review" => AnimeState::Thinking,
            _ => AnimeState::Thinking,
        }),
        UiEvent::StepDone { status, .. } => Some(match status.as_str() {
            "pass" | "done" => AnimeState::Success,
            "fail" | "blocked" | "refused" => AnimeState::Warning,
            "manual" => AnimeState::Curious,
            _ => AnimeState::Idle,
        }),
        UiEvent::ToolActivity { agent, .. } => Some(match agent.as_str() {
            "researcher" => AnimeState::Searching,
            "tester" | "tester/debugger" => AnimeState::Running,
            "coder" => AnimeState::Editing,
            "reviewer" => AnimeState::Thinking,
            _ => AnimeState::Thinking,
        }),
        UiEvent::ModelChunk { .. } => Some(AnimeState::Thinking),
        UiEvent::TestResult { passed, .. } => {
            Some(if *passed { AnimeState::Success } else { AnimeState::Error })
        }
        UiEvent::EditProposed { .. } => Some(AnimeState::WaitingPermission),
        UiEvent::EditApplied { .. } => Some(AnimeState::Editing),
        UiEvent::Review { passed, .. } => {
            Some(if *passed { AnimeState::Success } else { AnimeState::Warning })
        }
        UiEvent::AnswerStart { .. } => Some(AnimeState::Thinking),
        UiEvent::ModelDone { .. } => Some(AnimeState::Idle),
        UiEvent::AnswerDone { .. } => None,
        UiEvent::ModelSwitch { .. } => None,
        UiEvent::AgentComplete { green, .. } => {
            Some(if *green { AnimeState::Excited } else { AnimeState::Confused })
        }
    }
}

/// Face frames for a state (2–4 frames when animated, 1 otherwise).
pub fn frames(state: AnimeState) -> &'static [&'static [&'static str]] {
    match state {
        AnimeState::Idle => &[&["  ◉ ᴗ ◉  ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Listening => &[&["  ◎ ᴗ ◎  ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Thinking => &[
            &["  ◉ _ ◉  ", "  / | | \\ ", "    / \\   "],
            &["  ◉ _ ◉ .", "  / | | \\ ", "    / \\   "],
            &["  ◉ _ ◉..", "  / | | \\ ", "    / \\   "],
        ],
        AnimeState::Planning => &[&["  ◉ ‿ ◉ ✎", "  / | | \\ ", "    / \\   "]],
        AnimeState::Searching => &[
            &["  ◉ o ◉  ", "  / | | \\ ", "    / \\   "],
            &["  o ◉ ◉  ", "  / | | \\ ", "    / \\   "],
            &["  ◉ ◉ o  ", "  / | | \\ ", "    / \\   "],
        ],
        AnimeState::Reading => &[&["  ◉ - ◉  ", "  / |⌕| \\ ", "    / \\   "]],
        AnimeState::Coding => &[
            &["  ◉ # ◉  ", "  / | | \\ ", "    / \\   "],
            &["  ◉ # ◉ +", "  / | | \\ ", "    / \\   "],
            &["  ◉ # ◉++", "  / | | \\ ", "    / \\   "],
        ],
        AnimeState::Editing => &[&["  ◉ + ◉  ", "  / |✎| \\ ", "    / \\   "]],
        AnimeState::Running => &[
            &["  ◉ ▶ ◉  ", "  / | | \\ ", "    / \\   "],
            &["  ◉ ▷ ◉  ", "  / | | \\ ", "    / \\   "],
        ],
        AnimeState::WaitingPermission => &[&["  ◉ ? ◉  ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Success => &[&["  ★ ᴗ ★  ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Warning => &[&["  ◉ ~ ◉  ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Error => &[&["  ◉ ︵ ◉  ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Confused => &[&["  ◉ ¿ ◉  ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Curious => &[&["  ◉ ‿ ◉? ", "  / | | \\ ", "    / \\   "]],
        AnimeState::Excited => &[&["  ☆ ᴗ ☆  ", "  \\ | | / ", "    / \\   "]],
        AnimeState::Sleeping => &[&["  － ‿ －  ", "  / | | \\ ", "   z z z   "]],
    }
}

/// Rotating personality lines per state (the app cycles them slowly).
pub fn lines(state: AnimeState) -> &'static [&'static str] {
    match state {
        AnimeState::Idle => &["Ready when you are.", "Say the word."],
        AnimeState::Listening => &["I'm listening.", "Go on."],
        AnimeState::Thinking => &["Hmm… let me investigate.", "Thinking it through."],
        AnimeState::Planning => &["Drafting the plan.", "Mapping the moves."],
        AnimeState::Searching => &["Scanning the codebase…", "Hunting down the files…"],
        AnimeState::Reading => &["Reading the relevant files…", "Absorbing context…"],
        AnimeState::Coding => &["Writing the fix…", "Careful now…"],
        AnimeState::Editing => &["Applying the edits…", "Steady…"],
        AnimeState::Running => &["Running the tests…", "Holding my breath…"],
        AnimeState::WaitingPermission => &["Your call — allow?", "Waiting on you."],
        AnimeState::Success => &["Got it! Everything passed!", "Clean."],
        AnimeState::Warning => &["Heads up…", "Something needs a look."],
        AnimeState::Error => &["Oops… that didn't work.", "The tests are unhappy."],
        AnimeState::Confused => &["That one's tricky…", "Let me reconsider."],
        AnimeState::Curious => &["Interesting… tell me more.", "Needs a human eye."],
        AnimeState::Excited => &["Let's go!", "On it!"],
        AnimeState::Sleeping => &["Zzz… (idle)", "Wake me when needed."],
    }
}

/// Downgrade a face line to ASCII when the terminal lacks Unicode.
pub fn ascii_line(line: &str) -> String {
    line.chars()
        .map(|c| match c {
            '◉' | '◎' | '★' | '☆' => 'o',
            'ᴗ' | '‿' | '︵' | '~' | '¿' | '?' => '-',
            '／' => '/',
            '－' => '-',
            '▶' | '▷' => '>',
            '⌕' | '✎' | '#' | '+' => '*',
            '…' => '.',
            _ => c,
        })
        .collect()
}
