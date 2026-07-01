// Tests for the child launch argument builder (#881 --model passthrough).

use super::{ChildLaunchSpec, build_child_cli_args};
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
        disable_tools: Vec::new(),
    }
}

fn spec<'a>(config: &'a SubagentConfig) -> ChildLaunchSpec<'a> {
    ChildLaunchSpec {
        session_name: "w1",
        socket_path: Path::new("/run/w1.sock"),
        config,
        effective_config: None,
        parent_id: None,
        restrict_to_workspace: true,
        workflow_spec_path: None,
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
    let args = build_child_cli_args(&spec(&cfg));
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
    let args = build_child_cli_args(&spec(&cfg));
    let strs = as_strings(&args);
    assert!(
        !strs.iter().any(|a| a == "--model"),
        "no --model expected, got {strs:?}"
    );
}

#[test]
fn forwards_disable_tool_for_each_entry() {
    let mut cfg = base_config();
    cfg.disable_tools = vec!["write".into(), "edit".into()];
    let args = build_child_cli_args(&spec(&cfg));
    let strs = as_strings(&args);
    // Expect a `--disable-tool write` and a `--disable-tool edit` pair.
    let names: Vec<&str> = strs
        .iter()
        .zip(strs.iter().skip(1))
        .filter(|(a, _)| *a == "--disable-tool")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(names, vec!["write", "edit"], "got {strs:?}");
}

#[test]
fn omits_disable_tool_when_empty() {
    let cfg = base_config();
    let args = build_child_cli_args(&spec(&cfg));
    let strs = as_strings(&args);
    assert!(
        !strs.iter().any(|a| a == "--disable-tool"),
        "no --disable-tool expected, got {strs:?}"
    );
}

#[test]
fn forwards_existing_flags_alongside_model() {
    let mut cfg = base_config();
    cfg.system = Some("be terse".into());
    cfg.model = Some("anthropic/claude-sonnet-4-6".into());
    cfg.workflow = true;
    cfg.workflow_guards = true;
    let s = ChildLaunchSpec {
        session_name: "w1",
        socket_path: Path::new("/run/w1.sock"),
        config: &cfg,
        effective_config: Some(Path::new("/cfg.json")),
        parent_id: Some("parent-7"),
        restrict_to_workspace: false,
        workflow_spec_path: Some(Path::new("/run/spec.json")),
    };
    let args = build_child_cli_args(&s);
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
