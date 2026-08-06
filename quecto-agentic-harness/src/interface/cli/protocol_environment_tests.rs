//! #1369 slice 4 — `get_subagents` snapshots (`build_subagent_info_list`) carry
//! the execution backend and environment metadata so roster refreshes do not
//! lose what live events reported.

use super::*;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use std::path::PathBuf;

#[test]
fn build_subagent_info_list_carries_execution_backend_and_environment() {
    let env_registry = EnvironmentRegistry::new();
    env_registry.commit(EnvironmentRecord {
        environment_ref: "C1".to_string(),
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
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut entry = SubagentEntry::new(PathBuf::from("/tmp/impl.sock"), 7);
        entry.environment_registry = Some(env_registry);
        entry.environment_ref = Some("C1".to_string());
        guard.insert("impl".to_string(), entry);
        guard.insert(
            "solo".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/solo.sock"), 8),
        );
    }

    let list = build_subagent_info_list(&Some(reg));
    assert_eq!(list.len(), 2);
    let as_json = |info| serde_json::to_value::<&SubagentInfo>(info).unwrap();

    let scripted = as_json(&list[0]); // sorted: "impl" before "solo"
    assert_eq!(
        scripted.get("executionBackend").and_then(|v| v.as_str()),
        Some("script"),
        "snapshot entries for script-managed agents must carry the backend: {scripted}"
    );
    let env = scripted
        .get("environment")
        .expect("snapshot entries for script-managed agents must carry environment metadata");
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
        Some("direct")
    );

    let local = as_json(&list[1]);
    assert_eq!(
        local.get("executionBackend").and_then(|v| v.as_str()),
        Some("local"),
        "snapshot entries for local agents must report the local backend: {local}"
    );
    assert!(
        local.get("environment").is_none(),
        "local snapshot entries must not grow an environment object: {local}"
    );
}
