//! Region-coverage tests for `parse_agent_flags` / `validate_agent_flags`.
//!
//! Pure flag-parsing logic only — no config load, runtime, or socket I/O.
use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn mode_uds_sets_uds_mode() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--mode", "uds"]), &mut e).unwrap();
    assert!(f.uds_mode);
}

#[test]
fn mode_invalid_returns_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--mode", "bogus"]), &mut e).is_none());
    assert!(e.contains("not valid"));
}

#[test]
fn mode_missing_value_returns_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--mode"]), &mut e).is_none());
}

#[test]
fn socket_sets_path() {
    let mut e = String::new();
    let f =
        parse_agent_flags(&argv(&["--mode", "uds", "--socket", "/tmp/x.sock"]), &mut e).unwrap();
    assert_eq!(
        f.socket_path.as_deref(),
        Some(std::path::Path::new("/tmp/x.sock"))
    );
}

#[test]
fn socket_missing_value_returns_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--socket"]), &mut e).is_none());
}

#[test]
fn persist_requires_uds() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--persist", "-m", "hi"]), &mut e).is_none());
    assert!(e.contains("--persist requires --mode uds"));
}

#[test]
fn persist_with_uds_ok() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--mode", "uds", "--persist"]), &mut e).unwrap();
    assert!(f.persist && f.uds_mode);
}

#[test]
fn disable_tool_collects_multiple() {
    let mut e = String::new();
    let f = parse_agent_flags(
        &argv(&["--disable-tool", "bash", "--disable-tool", "edit"]),
        &mut e,
    )
    .unwrap();
    assert_eq!(
        f.disabled_tools,
        vec!["bash".to_string(), "edit".to_string()]
    );
}

#[test]
fn disable_tool_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--disable-tool"]), &mut e).is_none());
}

#[test]
fn effort_valid_parses() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--effort", "high"]), &mut e).unwrap();
    assert!(matches!(
        f.effort,
        Some(crate::domain::provider::EffortLevel::High)
    ));
}

#[test]
fn effort_invalid_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--effort", "ultra"]), &mut e).is_none());
    assert!(e.contains("invalid effort level"));
}

#[test]
fn effort_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--effort"]), &mut e).is_none());
}

#[test]
fn config_flag_consumes_value() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--config", "/tmp/c.toml", "-m", "hi"]), &mut e).unwrap();
    assert_eq!(f.message.as_deref(), Some("hi"));
}

#[test]
fn config_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--config"]), &mut e).is_none());
}

#[test]
fn workflow_spec_with_uds_sets_path() {
    let mut e = String::new();
    let f = parse_agent_flags(
        &argv(&["--mode", "uds", "--workflow-spec", "/tmp/w.toml"]),
        &mut e,
    )
    .unwrap();
    assert_eq!(
        f.workflow_spec_path.as_deref(),
        Some(std::path::Path::new("/tmp/w.toml"))
    );
}

#[test]
fn workflow_spec_requires_uds() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--workflow-spec", "/tmp/w.toml"]), &mut e).is_none());
    assert!(e.contains("require --mode uds"));
}

#[test]
fn workflow_spec_conflicts_no_workflow() {
    let mut e = String::new();
    let r = parse_agent_flags(
        &argv(&[
            "--mode",
            "uds",
            "--workflow-spec",
            "/tmp/w.toml",
            "--no-workflow",
        ]),
        &mut e,
    );
    assert!(r.is_none());
    assert!(e.contains("cannot be combined with --no-workflow"));
}

#[test]
fn parent_id_sets_value() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--parent-id", "root-1"]), &mut e).unwrap();
    assert_eq!(f.parent_id.as_deref(), Some("root-1"));
}

#[test]
fn parent_id_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--parent-id"]), &mut e).is_none());
}

#[test]
fn no_session_and_session_mutually_exclusive() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--no-session", "-s", "foo"]), &mut e).is_none());
    assert!(e.contains("mutually exclusive"));
}

#[test]
fn no_sandbox_flag_sets() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--no-sandbox"]), &mut e).unwrap();
    assert!(f.no_sandbox);
}

#[test]
fn workflow_guards_conflicts_no_workflow() {
    let mut e = String::new();
    let r = parse_agent_flags(
        &argv(&["--mode", "uds", "--no-workflow", "--workflow-guards"]),
        &mut e,
    );
    assert!(r.is_none());
    assert!(e.contains("cannot be used with --no-workflow"));
}

#[test]
fn workflow_and_guards_combo_in_uds() {
    let mut e = String::new();
    let f = parse_agent_flags(
        &argv(&["--mode", "uds", "--workflow", "--workflow-guards"]),
        &mut e,
    )
    .unwrap();
    assert!(f.workflow && f.workflow_guards && !f.workflow_disabled);
}
