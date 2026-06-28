// Tests for the child launch argument builder (#881 --model passthrough).

use super::build_child_cli_args;
use crate::domain::subagent::SubagentConfig;
use std::ffi::OsString;
use std::path::Path;

fn base_config() -> SubagentConfig {
    SubagentConfig {
        task: None,
        agent_id: Some("w1".into()),
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
    }
}

fn as_strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn includes_model_flag_when_set() {
    let mut cfg = base_config();
    cfg.model = Some("openai/gpt-5.5".into());
    let args = build_child_cli_args(
        "w1",
        Path::new("/run/w1.sock"),
        &cfg,
        None,
        None,
        true,
        None,
    );
    let strs = as_strings(&args);
    let pos = strs
        .iter()
        .position(|a| a == "--model")
        .expect("--model should be forwarded");
    assert_eq!(strs[pos + 1], "openai/gpt-5.5");
}

#[test]
fn omits_model_flag_when_absent() {
    let cfg = base_config();
    let args = build_child_cli_args(
        "w1",
        Path::new("/run/w1.sock"),
        &cfg,
        None,
        None,
        true,
        None,
    );
    let strs = as_strings(&args);
    assert!(
        !strs.iter().any(|a| a == "--model"),
        "no --model expected, got {strs:?}"
    );
}

#[test]
fn forwards_existing_flags_alongside_model() {
    let mut cfg = base_config();
    cfg.system = Some("be terse".into());
    cfg.model = Some("anthropic/claude-sonnet-4-6".into());
    cfg.workflow = true;
    cfg.workflow_guards = true;
    let args = build_child_cli_args(
        "w1",
        Path::new("/run/w1.sock"),
        &cfg,
        Some(Path::new("/cfg.json")),
        Some("parent-7"),
        false,
        Some(Path::new("/run/spec.json")),
    );
    let strs = as_strings(&args);
    for expected in [
        "--system",
        "--model",
        "--config",
        "--parent-id",
        "--workflow",
        "--workflow-guards",
        "--workflow-spec",
        "--no-sandbox",
    ] {
        assert!(
            strs.iter().any(|a| a == expected),
            "expected {expected} in {strs:?}"
        );
    }
}
