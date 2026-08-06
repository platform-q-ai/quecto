//! #1369 slice 4 — versioned wire DTOs carry the execution backend and
//! environment metadata for script-managed sub-agents, and forwarded merges
//! preserve them. Additive camelCase fields; local entries stay backward
//! compatible (`executionBackend: "local"`, no `environment` object).

use super::*;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
};
use crate::infrastructure::tools::subagent_registry::new_registry;
use std::path::PathBuf;

fn committed_env_entry(env_ref: &str, proxy: bool) -> SubagentEntry {
    let registry = EnvironmentRegistry::new();
    registry.commit(EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: "rt-9001".to_string(),
        environment_uuid: mint_environment_uuid(),
        name: Some("pr-env".to_string()),
        workspace_path: PathBuf::from("/work/pr-42"),
        repository: "https://example.com/acme/widget.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: Vec::new(),
        retained_kill_argv: Vec::new(),
        retained_cleanup_argv: Vec::new(),
        retained_inspect_argv: Vec::new(),
        members: vec!["uuid-impl".to_string()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({ "branch": "pr-42" }),
        last_error: None,
    });
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/impl.sock"), 7);
    entry.environment_registry = Some(registry);
    entry.environment_ref = Some(env_ref.to_string());
    if proxy {
        entry.proxy_bridge_socket = Some(PathBuf::from("/tmp/bridge.sock"));
    }
    entry
}

fn first_subagent(line: &str) -> serde_json::Value {
    let value: serde_json::Value = serde_json::from_str(line.trim()).expect("event parses");
    value
        .get("subagents")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .cloned()
        .expect("one subagent entry")
}

#[test]
fn state_changed_event_carries_execution_backend_and_environment() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("impl".to_string(), committed_env_entry("C1", true));

    let obj = first_subagent(&build_state_changed_event(&registry));
    assert_eq!(
        obj.get("executionBackend").and_then(|v| v.as_str()),
        Some("script"),
        "script-managed entries must report the script execution backend: {obj}"
    );
    let env = obj
        .get("environment")
        .expect("script-managed entries must carry an environment object");
    assert_eq!(env.get("ref").and_then(|v| v.as_str()), Some("C1"));
    assert_eq!(env.get("name").and_then(|v| v.as_str()), Some("pr-env"));
    assert_eq!(env.get("status").and_then(|v| v.as_str()), Some("running"));
    assert_eq!(
        env.get("repository").and_then(|v| v.as_str()),
        Some("https://example.com/acme/widget.git")
    );
    assert_eq!(env.get("branch").and_then(|v| v.as_str()), Some("pr-42"));
    assert_eq!(
        env.get("runtimeId").and_then(|v| v.as_str()),
        Some("rt-9001")
    );
    assert_eq!(
        env.get("workspace").and_then(|v| v.as_str()),
        Some("/work/pr-42")
    );
    assert_eq!(
        env.get("socketMode").and_then(|v| v.as_str()),
        Some("proxy")
    );
}

#[test]
fn state_changed_event_reports_direct_socket_mode_without_proxy_bridge() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("impl".to_string(), committed_env_entry("C1", false));

    let obj = first_subagent(&build_state_changed_event(&registry));
    let env = obj.get("environment").expect("environment object");
    assert_eq!(
        env.get("socketMode").and_then(|v| v.as_str()),
        Some("direct"),
        "entries without a proxy bridge must report the direct socket mode: {obj}"
    );
}

#[test]
fn state_changed_event_marks_local_entries_as_local_backend() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "solo".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/solo.sock"), 7),
    );

    let obj = first_subagent(&build_state_changed_event(&registry));
    assert_eq!(
        obj.get("executionBackend").and_then(|v| v.as_str()),
        Some("local"),
        "local entries must report the local execution backend: {obj}"
    );
    assert!(
        obj.get("environment").is_none(),
        "local entries must not grow an environment object: {obj}"
    );
}

#[test]
fn forwarded_descendant_merge_preserves_environment_metadata() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "child".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/child.sock"), 1),
    );
    let event = serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": [{
            "agentId": "grand",
            "agentUuid": "uuid-grand",
            "displayName": "grand",
            "parentId": "child",
            "status": "running",
            "pid": 42,
            "socketPath": "/tmp/grand.sock",
            "readOnly": false,
            "executionBackend": "script",
            "environment": {
                "ref": "C3",
                "status": "running",
                "repository": "https://example.com/acme/widget.git",
                "runtimeId": "rt-42",
                "workspace": "/work/c3",
                "socketMode": "direct",
            },
        }],
    });

    let forwarded =
        crate::infrastructure::tools::subagent_monitor_merge::merge_and_forward_state_changed(
            &event, &registry, "child",
        )
        .expect("state_changed events forward");
    let value: serde_json::Value = serde_json::from_str(forwarded.trim()).unwrap();
    let grand = value
        .get("subagents")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
        .find(|s| s.get("agentUuid").and_then(|v| v.as_str()) == Some("uuid-grand"))
        .cloned()
        .expect("forwarded union must include the merged grandchild");
    assert_eq!(
        grand.get("executionBackend").and_then(|v| v.as_str()),
        Some("script"),
        "the merged re-broadcast must preserve the execution backend: {grand}"
    );
    let env = grand
        .get("environment")
        .expect("the merged re-broadcast must preserve the environment object");
    assert_eq!(env.get("ref").and_then(|v| v.as_str()), Some("C3"));
    assert_eq!(env.get("runtimeId").and_then(|v| v.as_str()), Some("rt-42"));
    assert_eq!(
        env.get("socketMode").and_then(|v| v.as_str()),
        Some("direct")
    );
}
