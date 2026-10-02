//! TUI tests — event bus, anime engine, themes. All offline, no terminal.

use local_ai::core::ui_events as ev;
use local_ai::tui::{anime, theme};

// The approval hook is process-global: serialize tests that touch it.
static HOOK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

#[test]
fn test_bus_emit_receive_round_trip() {
    let mut rx = ev::subscribe();
    ev::emit(ev::UiEvent::Log { line: "hello".into() });
    match rx.try_recv() {
        Ok(ev::UiEvent::Log { line }) => assert_eq!(line, "hello"),
        other => panic!("unexpected: {:?}", other.is_ok()),
    }
}

#[test]
fn test_bus_no_subscriber_is_noop() {
    // Must never panic or block when nobody listens.
    ev::emit(ev::UiEvent::AgentComplete { green: true, summary: "x".into() });
}

#[test]
fn test_takeover_flag_round_trip() {
    let prev = ev::ui_takeover();
    ev::set_ui_takeover(true);
    assert!(ev::ui_takeover());
    ev::set_ui_takeover(prev);
}

#[test]
fn test_approval_hook_absent_by_default() {
    let _guard = HOOK_LOCK.lock().unwrap();
    ev::set_approval_hook(None);
    assert!(!ev::has_approval_hook());
    // No hook → request_approval resolves to None immediately (CLI path).
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let out = rt.block_on(ev::request_approval("p".into(), vec![]));
    assert!(out.is_none());
}

#[test]
fn test_approval_round_trip_through_hook() {
    let _guard = HOOK_LOCK.lock().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    ev::set_approval_hook(Some(tx));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let choice = rt.block_on(async {
        let waiter = tokio::spawn(async {
            ev::request_approval("apply?".into(), vec!["+x".into()]).await
        });
        let req = rx.recv().await.expect("request arrives");
        assert_eq!(req.prompt, "apply?");
        assert_eq!(req.diff, vec!["+x".to_string()]);
        req.respond.send(ev::ApprovalChoice::Session).unwrap();
        waiter.await.unwrap()
    });
    assert_eq!(choice, Some(ev::ApprovalChoice::Session));
    ev::set_approval_hook(None);
}

// ---------------------------------------------------------------------------
// Anime engine
// ---------------------------------------------------------------------------

#[test]
fn test_every_event_maps_without_panic() {
    let events = vec![
        ev::UiEvent::Log { line: String::new() },
        ev::UiEvent::Warn { line: String::new() },
        ev::UiEvent::AgentStart { goal: "g".into(), steps: 3, max_steps: 20 },
        ev::UiEvent::PlanCreated { steps: vec![] },
        ev::UiEvent::StepStart { id: "s".into(), title: "t".into(), kind: "edit".into() },
        ev::UiEvent::StepDone { id: "s".into(), status: "done".into() },
        ev::UiEvent::ToolActivity { agent: "coder".into(), action: "a".into(), detail: "d".into(), status: "s".into() },
        ev::UiEvent::ModelChunk { text: "hi".into() },
        ev::UiEvent::TestResult { step: "t".into(), passed: true, summary: "s".into() },
        ev::UiEvent::EditProposed { step: "e".into(), files: vec![], diff: vec![] },
        ev::UiEvent::EditApplied { step: "e".into(), applied: vec![] },
        ev::UiEvent::Review { passed: false, summary: "s".into() },
        ev::UiEvent::AnswerStart { query: "q".into() },
        ev::UiEvent::ModelDone { full: "f".into() },
        ev::UiEvent::AgentComplete { green: true, summary: "s".into() },
    ];
    for ev in &events {
        let _ = anime::state_for_event(ev);
    }
    // Log/Warn never steal the character.
    assert!(anime::state_for_event(&events[0]).is_none());
    assert!(anime::state_for_event(&events[1]).is_none());
}

#[test]
fn test_all_states_have_frames_and_lines() {
    use anime::AnimeState;
    let states = [
        AnimeState::Idle, AnimeState::Listening, AnimeState::Thinking, AnimeState::Planning,
        AnimeState::Searching, AnimeState::Reading, AnimeState::Coding, AnimeState::Editing,
        AnimeState::Running, AnimeState::WaitingPermission, AnimeState::Success,
        AnimeState::Warning, AnimeState::Error, AnimeState::Confused, AnimeState::Curious,
        AnimeState::Excited, AnimeState::Sleeping,
    ];
    for s in states {
        assert!(!anime::frames(s).is_empty(), "{:?}", s);
        assert!(!anime::lines(s).is_empty(), "{:?}", s);
        for frame in anime::frames(s) {
            assert!(!frame.is_empty());
        }
    }
}

#[test]
fn test_ascii_faces_are_pure_ascii() {
    use anime::AnimeState;
    let states = [
        AnimeState::Idle, AnimeState::Thinking, AnimeState::Success, AnimeState::Error,
        AnimeState::Sleeping, AnimeState::Excited, AnimeState::Running,
    ];
    for s in states {
        for frame in anime::frames(s) {
            for row in *frame {
                let ascii = anime::ascii_line(row);
                assert!(ascii.is_ascii(), "{} → {}", row, ascii);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Themes
// ---------------------------------------------------------------------------

#[test]
fn test_builtin_themes_load() {
    for name in theme::builtin_names() {
        let t = theme::builtin(name);
        assert_eq!(t.name, *name);
    }
    // Unknown names fall back to midnight, never panic.
    assert_eq!(theme::builtin("nope").name, "midnight");
}

#[test]
fn test_theme_json_round_trip() {
    let t = theme::builtin("matrix");
    let json = serde_json::to_string(&t).unwrap();
    assert!(json.contains("matrix"));
    let back: theme::Theme = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "matrix");
}

#[test]
fn test_caps_respect_no_color_env() {
    let prev = std::env::var("NO_COLOR").ok();
    std::env::set_var("NO_COLOR", "1");
    let caps = theme::detect_caps(false, false);
    assert!(!caps.colors);
    if let Some(v) = prev {
        std::env::set_var("NO_COLOR", v);
    } else {
        std::env::remove_var("NO_COLOR");
    }
}
