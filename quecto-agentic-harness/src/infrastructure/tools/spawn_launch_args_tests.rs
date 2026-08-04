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
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
        container: crate::domain::container_runtime::SpawnContainerRequest::Local,
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
        inherited_tool_policy_path: None,
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
fn includes_effort_flag_when_set() {
    let mut cfg = base_config();
    cfg.effort = Some("high".into());
    let args = build_child_cli_args(&spec(&cfg));
    let strs = as_strings(&args);
    let pos = strs
        .iter()
        .position(|a| a == "--effort")
        .expect("--effort should be forwarded");
    assert_eq!(strs[pos + 1], "high");
}

#[test]
fn omits_effort_flag_when_absent() {
    let cfg = base_config();
    let args = build_child_cli_args(&spec(&cfg));
    let strs = as_strings(&args);
    assert!(
        !strs.iter().any(|a| a == "--effort"),
        "no --effort expected, got {strs:?}"
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
        inherited_tool_policy_path: None,
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
        "--spawned",
    ] {
        assert!(
            strs.iter().any(|a| a == expected),
            "expected {expected} in {strs:?}"
        );
    }
}

/// #1319: every SpawnTool child launch must carry the explicit internal flag.
#[test]
fn always_emits_spawned_flag() {
    let cfg = base_config();
    let args = build_child_cli_args(&spec(&cfg));
    let strs = as_strings(&args);
    assert!(
        strs.iter().any(|a| a == "--spawned"),
        "SpawnTool children must always receive --spawned; got {strs:?}"
    );
}

/// #1319: --spawned is independent of --parent-id (present or absent).
#[test]
fn emits_spawned_without_parent_id() {
    let cfg = base_config();
    let mut s = spec(&cfg);
    s.parent_id = None;
    let strs = as_strings(&build_child_cli_args(&s));
    assert!(strs.iter().any(|a| a == "--spawned"));
    assert!(!strs.iter().any(|a| a == "--parent-id"));
}

#[test]
fn child_launch_args_include_internal_inherited_tool_policy_snapshot_flag() {
    let cfg = base_config();
    let mut s = spec(&cfg);
    s.inherited_tool_policy_path = Some(Path::new("/run/policy.json"));

    let args = build_child_cli_args(&s);
    let strs = as_strings(&args);
    let pos = strs
        .iter()
        .position(|arg| arg == "--inherited-tool-policy-snapshot")
        .expect("policy snapshot flag is forwarded");
    assert_eq!(strs[pos + 1], "/run/policy.json");
}

/// #1378: `-s` is the durable session key (AgentUuid), not the display label.
/// The child's `Session::build_key("cli", name)` therefore cannot resume a
/// previous label-named session when the same display name is reused.
#[test]
fn child_session_flag_uses_uuid_key_not_display_label() {
    let cfg = base_config();
    let uuid = "11111111-2222-4333-8444-555555555555";
    let socket = format!("/run/quecto-agent-{uuid}.sock");
    let s = ChildLaunchSpec {
        session_name: uuid,
        socket_path: Path::new(&socket),
        config: &cfg,
        effective_config: None,
        parent_id: None,
        restrict_to_workspace: true,
        workflow_spec_path: None,
        inherited_tool_policy_path: None,
    };
    let strs = as_strings(&build_child_cli_args(&s));
    let s_pos = strs
        .iter()
        .position(|a| a == "-s")
        .expect("-s session flag must be present");
    assert_eq!(strs[s_pos + 1], uuid, "session key must be the AgentUuid");
    assert!(
        !strs.iter().any(|a| a == "w1" || a == "reviewer"),
        "display labels must not appear in child CLI args, got {strs:?}"
    );
    let sock_pos = strs
        .iter()
        .position(|a| a == "--socket")
        .expect("--socket must be present");
    assert_eq!(strs[sock_pos + 1], socket);
}
