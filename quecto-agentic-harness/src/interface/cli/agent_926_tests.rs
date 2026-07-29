//! #926: a spawn-capable parent must ALWAYS have a live notification receiver
//! paired with the `notify_tx` wired into `SpawnTool`. The previous wiring
//! dropped `notify_rx` (set `notification_rx: None`) whenever `base_dir` was
//! empty, even though `SpawnTool` (and its `notify_tx`) is registered
//! unconditionally — so a child's completion was emitted into a channel with no
//! receiver and the idle parent was never woken.
use super::*;
use crate::infrastructure::config::Config;

fn config_with_provider() -> Config {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    Config::load(tmp.path().to_str().unwrap()).unwrap()
}

fn spawn_capable_flags() -> AgentFlags {
    AgentFlags {
        session_name: None,
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: true,
        no_sandbox: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        parent_id: None,
        spawned: false,
    }
}

/// The wiring invariant: because `SpawnTool` (with `notify_tx`) is registered
/// unconditionally, the build MUST hand back a live `notification_rx` so the
/// dispatch loop can wake on completions. A `None` receiver here is the #926
/// wake gap (a live sender with a dropped receiver).
#[test]
fn test_926_spawn_capable_build_has_live_notification_rx_with_real_base_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_with_provider();
    let flags = spawn_capable_flags();
    let mut stderr = String::new();
    let build = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config: &config,
        http_client: &reqwest::Client::new(),
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: None,
    })
    .expect("registry build should succeed");
    assert!(
        build.notification_rx.is_some(),
        "spawn-capable parent must have a live notification receiver"
    );
    assert!(
        build.subagent_registry.is_some(),
        "spawn-capable parent must expose the protocol registry"
    );
}

/// Regression for the exact gate that caused #926: even with an EMPTY base_dir
/// the `notify_tx` is still wired into `SpawnTool`, so the receiver must stay
/// live — a dropped rx here is the silent wake gap. This FAILS against the old
/// `if has_base_dir { Some(notify_rx) } else { None }` wiring.
#[test]
fn test_926_empty_base_dir_still_keeps_notification_rx_live() {
    let config = config_with_provider();
    let flags = spawn_capable_flags();
    let mut stderr = String::new();
    let cwd = tempfile::TempDir::new().unwrap();
    let build = build_tool_registry(ToolRegistryArgs {
        base_dir: std::path::Path::new(""),
        config: &config,
        http_client: &reqwest::Client::new(),
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: cwd.path(),
        home_dir: None,
    })
    .expect("registry build should succeed");
    assert!(
        build.notification_rx.is_some(),
        "a live notify_tx must never be paired with a dropped notify_rx (#926)"
    );
}

/// #957: the END STATE of a read-only child. A child launched with
/// `--disable-tool write --disable-tool edit` (what a `read_only: true` spawn
/// forwards) must build a registry with `write`/`edit` removed — the model never
/// sees them — while its non-mutating toolset (`bash`/`read`/`grep`/`find`/
/// `agent_cmd`) stays intact. Asserts the registry end-state directly rather than
/// inferring it transitively from the `remove_all` unit tests.
#[test]
fn test_957_read_only_child_registry_omits_write_edit_keeps_others() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_with_provider();
    let mut flags = spawn_capable_flags();
    flags.disabled_tools = vec!["write".to_string(), "edit".to_string()];
    let mut stderr = String::new();
    let mut build = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config: &config,
        http_client: &reqwest::Client::new(),
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: None,
    })
    .expect("registry build should succeed");
    // Guard against a vacuous pass: the base registry must actually expose the
    // tools we intend to remove, so the post-removal absence proves removal (not
    // that they were never registered — the exact regression a read-only guard
    // must catch).
    let before = build.registry.names();
    for t in ["write", "edit"] {
        assert!(
            before.contains(&t.to_string()),
            "base registry must expose `{t}` before removal; names = {before:?}"
        );
    }
    // Mirror the child CLI: tools named on `--disable-tool` are removed before
    // the session starts (agent.rs applies `registry.remove_all`).
    build.registry.remove_all(&flags.disabled_tools);
    let names = build.registry.names();
    for gone in ["write", "edit"] {
        assert!(
            !names.contains(&gone.to_string()),
            "read-only child must not expose `{gone}`; names = {names:?}"
        );
    }
    for kept in ["bash", "read", "grep", "find", "agent_cmd"] {
        assert!(
            names.contains(&kept.to_string()),
            "read-only child must retain `{kept}`; names = {names:?}"
        );
    }
}
