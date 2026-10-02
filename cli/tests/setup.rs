//! First-run setup tests — model pick logic, config keys, isolated save.
//!
//! Provider-network paths are verified live (`init`, `provider`, `doctor`);
//! these tests cover the offline-decidable rules. Config-dir isolation uses
//! `XDG_CONFIG_HOME` (serialized: env is process-global).

use local_ai::core::config::ProviderKind;

static CFG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run `f` with `XDG_CONFIG_HOME` pointed at a fresh temp dir.
fn with_temp_config(f: impl FnOnce()) {
    let _guard = CFG_LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("local-ai-cfg-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let prev = std::env::var("XDG_CONFIG_HOME").ok();
    std::env::set_var("XDG_CONFIG_HOME", &dir);
    f();
    match prev {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// pick_model: explicit > saved-when-listed > first > none
// ---------------------------------------------------------------------------

#[test]
fn test_pick_model_priority() {
    use local_ai::core::provider::pick_model;
    let avail = vec!["a".to_string(), "b".to_string()];
    // Explicit always wins, even when not listed (offline trust).
    assert_eq!(pick_model(Some("custom"), Some("a"), &avail).as_deref(), Some("custom"));
    assert_eq!(pick_model(Some("custom"), None, &[]).as_deref(), Some("custom"));
    // Saved model wins when listed.
    assert_eq!(pick_model(None, Some("b"), &avail).as_deref(), Some("b"));
    // Stale saved model degrades to first available, never fails.
    assert_eq!(pick_model(None, Some("gone"), &avail).as_deref(), Some("a"));
    // Nothing anywhere → None (caller goes manual/offline).
    let empty: Vec<String> = Vec::new();
    assert_eq!(pick_model(None, None, &empty), None);
    assert_eq!(pick_model(None, Some("x"), &empty), None);
    assert_eq!(pick_model(Some(""), Some("a"), &avail).as_deref(), Some("a"));
}

// ---------------------------------------------------------------------------
// resolve_model_id: explicit path never touches the network
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resolve_explicit_needs_no_provider() {
    use local_ai::core::provider::resolve_model_id;
    // Unreachable URL + explicit id → returns id with zero I/O.
    let cfg = local_ai::core::config::default_config();
    let out = resolve_model_id(
        Some("my-model".into()),
        &ProviderKind::Generic,
        "http://127.0.0.1:9",
        &cfg,
    )
    .await;
    assert_eq!(out.as_deref(), Some("my-model"));
}

// ---------------------------------------------------------------------------
// config model key + first-run/save round trip (isolated dir)
// ---------------------------------------------------------------------------

#[test]
fn test_config_model_defaults_none() {
    let cfg = local_ai::core::config::default_config();
    assert!(cfg.model.is_none());
    // Serde keeps old files readable (missing `model` key → None).
    let parsed: local_ai::core::config::AppConfig = toml::from_str("").unwrap();
    assert!(parsed.model.is_none());
    assert_eq!(parsed.mode, local_ai::core::config::AppMode::Build);
}

#[test]
fn test_first_run_and_save_selection_isolated() {
    with_temp_config(|| {
        use local_ai::core::setup;
        assert!(setup::is_first_run(), "fresh dir is first run");
        setup::save_selection(ProviderKind::Ollama, Some("qwen3-coder".into())).unwrap();
        assert!(!setup::is_first_run(), "saved file ends first run");
        let cfg = local_ai::core::config::load_config().unwrap();
        assert_eq!(cfg.provider.active, ProviderKind::Ollama);
        assert_eq!(cfg.model.as_deref(), Some("qwen3-coder"));
        // Clearing the model keeps the provider (launch re-picks inline).
        setup::save_selection(ProviderKind::Ollama, None).unwrap();
        let cfg = local_ai::core::config::load_config().unwrap();
        assert!(cfg.model.is_none());
    });
}

#[test]
fn test_provider_options_cover_big_three() {
    let cfg = local_ai::core::config::default_config();
    let opts = local_ai::core::setup::provider_options(&cfg);
    let kinds: Vec<ProviderKind> =
        opts.into_iter().map(|o| o.kind).collect();
    assert!(kinds.contains(&ProviderKind::LmStudio));
    assert!(kinds.contains(&ProviderKind::Ollama));
    assert!(kinds.contains(&ProviderKind::LlamaCpp));
}
