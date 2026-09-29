//! The claude-code member runner (#2287): `cmd_agent` routes `--backend
//! claude-code` here after admission; the runner holds composition's
//! handles only.

use super::super::agent::{cmd_agent, parse_agent_flags};
use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

const NOT_COMPOSED: &str = "agent: the claude-code member capability is not composed\n";

#[test]
fn an_uncomposed_member_capability_is_refused() {
    let mut stderr = String::new();
    let flags = parse_agent_flags(
        &argv(&["--mode", "uds", "--backend", "claude-code"]),
        &mut stderr,
    )
    .expect("valid flags");
    assert_eq!(run(&CliContext::default(), &flags, &mut stderr), 1);
    assert_eq!(stderr, NOT_COMPOSED);
}

/// #2287 review (L3): outside a swarm run, `cmd_agent` refuses the
/// claude-code backend after admission, before the runner.
#[test]
fn cmd_agent_refuses_a_claude_code_member_outside_a_swarm() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        cwd: Some(tmp.path().to_path_buf()),
        claude_member: Some(|_| panic!("the runner is never reached")),
        ..Default::default()
    };
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = cmd_agent(
        &ctx,
        &argv(&["--mode", "uds", "--backend", "claude-code", "-s", "w1"]),
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(
        (code, stderr.as_str()),
        (
            1,
            "agent: --backend claude-code runs only as a swarm worker admitted to a created swarm run\n"
        )
    );
}

/// #2287 review (L1): a member composition refuses to build is refused
/// with its reason, before anything runs.
#[test]
fn a_member_composition_refuses_is_refused_with_its_reason() {
    let mut stderr = String::new();
    let flags = parse_agent_flags(
        &argv(&["--mode", "uds", "--backend", "claude-code"]),
        &mut stderr,
    )
    .expect("valid flags");
    let ctx = CliContext {
        claude_member: Some(|_| Err("no credential for you".to_string())),
        ..Default::default()
    };
    assert_eq!(run(&ctx, &flags, &mut stderr), 1);
    assert_eq!(stderr, "agent: no credential for you\n");
}

fn log_key_for(args: &[&str]) -> Option<String> {
    let mut stderr = String::new();
    let mut parts = vec!["--mode", "uds", "--backend", "claude-code"];
    parts.extend_from_slice(args);
    let flags = parse_agent_flags(&argv(&parts), &mut stderr).expect("valid flags");
    member_settings(&CliContext::default(), &flags).log_key
}

/// #2304 review round 3: a member asked to leave nothing behind
/// (`--no-session`, `-s -`) keeps no event log.
#[test]
fn a_member_asked_to_leave_nothing_behind_keeps_no_log() {
    assert_eq!(log_key_for(&["--no-session"]), None);
    assert_eq!(log_key_for(&["-s", "-"]), None);
}

/// #2304 review round 3: a named member's log is its session's; an
/// unnamed one's is its own, never another unnamed member's.
#[test]
fn each_unnamed_member_keeps_a_log_of_its_own() {
    assert_eq!(log_key_for(&["-s", "w1"]).as_deref(), Some("cli:w1"));
    let unnamed = log_key_for(&[]).expect("an unnamed member keeps a log");
    let own = format!("unkeyed-{}-", std::process::id());
    assert!(unnamed.starts_with(&own), "{unnamed}");
    assert_ne!(unnamed, "cli:claude-member");
}
