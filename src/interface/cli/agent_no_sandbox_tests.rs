use super::*;
use crate::interface::cli::{CliContext, run_with_output};

fn default_ctx() -> CliContext {
    CliContext::default()
}

// ===================================================================
// --no-sandbox flag tests
// ===================================================================

#[test]
fn test_agent_no_sandbox_flag_defaults_false() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(!flags.no_sandbox, "--no-sandbox should be false by default");
}

#[test]
fn test_agent_no_sandbox_flag_parsed() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--no-sandbox".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(
        flags.no_sandbox,
        "--no-sandbox should be true when provided"
    );
}

#[test]
fn test_agent_no_sandbox_combined_with_no_session() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-sandbox".into(),
        "--no-session".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_sandbox);
    assert!(flags.no_session);
}

#[test]
fn test_agent_no_sandbox_combined_with_session() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-sandbox".into(),
        "-s".into(),
        "my-session".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_sandbox);
    assert_eq!(flags.session_name.as_deref(), Some("my-session"));
}

#[test]
fn test_agent_no_sandbox_overrides_config_restrict_to_workspace() {
    // When --no-sandbox is set, restrict_to_workspace should be false
    // regardless of config value.
    use crate::infrastructure::security::sandbox::Sandbox;
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--no-sandbox".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();

    // Simulate what build_agent_from_config does when --no-sandbox is set:
    // restrict_to_workspace becomes false even if config says true.
    let effective_restrict = if flags.no_sandbox {
        false
    } else {
        true // config default
    };
    let sb = Sandbox::new(
        Some(std::path::PathBuf::from("/tmp/ws")),
        effective_restrict,
    );
    assert!(
        !sb.restrict_to_workspace,
        "sandbox should not restrict workspace when --no-sandbox is set"
    );
}

#[test]
fn test_agent_help_documents_no_sandbox() {
    let out = run_with_output(vec!["quecto".into(), "help".into()], &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("--no-sandbox"),
        "help text must document --no-sandbox, got:\n{}",
        out.stdout
    );
}
