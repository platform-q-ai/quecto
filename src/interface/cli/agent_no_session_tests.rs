// Tests for --no-session flag (Issue #191).
use super::*;

#[test]
fn test_agent_no_session_flag_parsed() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--no-session".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_session, "expected no_session to be true");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn test_agent_no_session_and_s_are_mutually_exclusive() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-session".into(),
        "-s".into(),
        "mysession".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(
        result.is_none(),
        "expected None when --no-session and -s are combined"
    );
    assert!(
        stderr.contains("mutually exclusive"),
        "expected 'mutually exclusive' in stderr: {stderr}",
    );
}

#[test]
fn test_agent_no_session_default_is_false() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(
        !flags.no_session,
        "expected no_session to be false by default"
    );
}

#[test]
fn test_agent_no_session_leaves_session_name_none() {
    // --no-session sets no_session=true and session_name stays None
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--no-session".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_session);
    assert!(flags.session_name.is_none());
}

#[test]
fn test_agent_s_dash_still_works_as_ephemeral_alias() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-s".into(), "-".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("-"));
    assert!(!flags.no_session);
}

#[test]
fn test_agent_no_session_combined_with_other_flags() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-session".into(),
        "--model".into(),
        "gpt-4o".into(),
        "-m".into(),
        "Hello".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_session);
    assert_eq!(flags.model_override.as_deref(), Some("gpt-4o"));
    assert_eq!(flags.message.as_deref(), Some("Hello"));
    assert!(stderr.is_empty());
}

// ── #881: spawned `model` reaches the child and runs on it before any turn ──
//
// These cover AC1 (the child adopts the requested model, verifiable before the
// first turn) and AC4 (precedence: explicit model > --config default) at the
// resolution boundary the child actually uses. The complementary half — that
// `spawn` emits `--model <model>` — is covered by `spawn_launch_args_tests.rs`.

#[test]
fn spawned_model_flag_parses_into_model_override() {
    // Mirror the exact argv a spawned child receives from spawn's launch
    // builder (`build_child_cli_args`): UDS mode, the forwarded `--model`, and a
    // `--config` whose own default differs. The child's parser must surface the
    // spawned model as `model_override`.
    let mut stderr = String::new();
    let argv: Vec<String> = vec![
        "agent".into(),
        "--mode".into(),
        "uds".into(),
        "-s".into(),
        "child-session".into(),
        "--persist".into(),
        "--model".into(),
        "openai/spawned-model".into(),
        "--config".into(),
        "/tmp/child-config.toml".into(),
    ];
    let flags = parse_agent_flags(&argv, &mut stderr).unwrap();
    assert_eq!(
        flags.model_override.as_deref(),
        Some("openai/spawned-model"),
        "child must parse the spawned --model into model_override; stderr={stderr}"
    );
}

#[test]
fn resolve_agent_model_prefers_explicit_over_config_default() {
    // AC4: an explicit model (e.g. a spawned `--model`) wins over the
    // config-supplied default. This is the resolution the child performs before
    // its first turn, so get_state/get_session_stats report the spawned model.
    use super::agent_tool_registry::resolve_agent_model;
    assert_eq!(
        resolve_agent_model(Some("openai/spawned-model"), "anthropic/config-default"),
        "openai/spawned-model"
    );
}

#[test]
fn resolve_agent_model_falls_back_to_config_default_when_absent() {
    // AC2: omitting the model preserves prior behaviour — the config default is
    // used unchanged.
    use super::agent_tool_registry::resolve_agent_model;
    assert_eq!(
        resolve_agent_model(None, "anthropic/config-default"),
        "anthropic/config-default"
    );
}

#[test]
fn spawned_model_end_to_end_overrides_config_default() {
    // AC1 + AC4 end-to-end across the child's real entry path: parse the argv a
    // spawned child gets, then resolve the effective model against a *different*
    // config default. The spawned model must win — i.e. the child runs on it.
    let mut stderr = String::new();
    let argv: Vec<String> = vec![
        "agent".into(),
        "--mode".into(),
        "uds".into(),
        "-s".into(),
        "child-session".into(),
        "--persist".into(),
        "--model".into(),
        "openai/spawned-model".into(),
    ];
    let flags = parse_agent_flags(&argv, &mut stderr).unwrap();
    let effective = super::agent_tool_registry::resolve_agent_model(
        flags.model_override.as_deref(),
        "anthropic/config-default",
    );
    assert_eq!(effective, "openai/spawned-model");
}
