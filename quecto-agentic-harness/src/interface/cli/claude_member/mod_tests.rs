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

#[test]
fn cmd_agent_routes_the_claude_code_backend_to_the_member_runner() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        cwd: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = cmd_agent(
        &ctx,
        &argv(&["--mode", "uds", "--backend", "claude-code", "-s", "w1"]),
        &mut stdout,
        &mut stderr,
    );
    assert_eq!((code, stderr.as_str()), (1, NOT_COMPOSED));
}
