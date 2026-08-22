use super::*;
use serde_json::json;

#[test]
fn tool_catalogue_changed_event_serializes_additive_shape() {
    let event = AgentEvent::ToolCatalogueChanged {
        changed_tools: vec!["alpha".to_string()],
        before: vec![json!({"name":"alpha","effectiveEnabled":true})],
        after: vec![json!({"name":"alpha","effectiveEnabled":false})],
        reason: "reload".to_string(),
    };
    let value = serde_json::to_value(event).unwrap();
    assert_eq!(value["type"], "tool_catalogue_changed");
    assert_eq!(value["changedTools"], json!(["alpha"]));
    assert_eq!(value["before"][0]["effectiveEnabled"], true);
    assert_eq!(value["after"][0]["effectiveEnabled"], false);
    assert_eq!(value["reason"], "reload");
}

#[test]
fn tool_policy_changed_event_serializes_apply_mode_and_results() {
    let event = AgentEvent::ToolPolicyChanged {
        changed_tools: vec!["alpha".to_string()],
        results: vec![
            json!({"name":"alpha","status":"applied","after":{"name":"alpha","profileScope":"child","effectiveScope":"child","effectiveParentEnabled":false,"effectiveChildEnabled":true}}),
        ],
        apply_mode: "atNextTurnBoundary".to_string(),
        reason: "queued".to_string(),
        correlation_id: Some("req-1".to_string()),
    };
    let value = serde_json::to_value(event).unwrap();
    assert_eq!(value["type"], "tool_policy_changed");
    assert_eq!(value["changedTools"], json!(["alpha"]));
    assert_eq!(value["applyMode"], "atNextTurnBoundary");
    assert_eq!(value["results"][0]["status"], "applied");
    assert_eq!(value["results"][0]["after"]["profileScope"], "child");
    assert_eq!(value["results"][0]["after"]["effectiveScope"], "child");
    assert_eq!(value["reason"], "queued");
    assert_eq!(value["correlationId"], "req-1");
}

#[test]
fn build_subagent_info_list_reports_liveness_without_changing_legacy_status() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut live = SubagentEntry::new("/tmp/live.sock".into(), 10);
        live.status = SubagentStatus::Idle;
        guard.insert("live".to_string(), live);
        let mut dead = SubagentEntry::new("/tmp/dead.sock".into(), 11);
        dead.status = SubagentStatus::Exited;
        dead.persisted_liveness = crate::domain::session::SubagentLiveness::Dead;
        guard.insert("dead".to_string(), dead);
    }

    let list = build_subagent_info_list(&Some(reg));
    let live = list.iter().find(|info| info.agent_id == "live").unwrap();
    assert_eq!(live.status, "idle");
    assert_eq!(live.liveness.as_deref(), Some("live"));
    let dead = list.iter().find(|info| info.agent_id == "dead").unwrap();
    assert_eq!(dead.status, "exited");
    assert_eq!(dead.liveness.as_deref(), Some("dead"));

    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut detached = SubagentEntry::new("/tmp/detached.sock".into(), 12);
        detached.status = SubagentStatus::Exited;
        detached.persisted_liveness = crate::domain::session::SubagentLiveness::Detached;
        guard.insert("detached".to_string(), detached);
    }
    let list = build_subagent_info_list(&Some(reg));
    let detached = list
        .iter()
        .find(|info| info.agent_id == "detached")
        .unwrap();
    assert_eq!(detached.status, "exited");
    assert_eq!(detached.liveness.as_deref(), Some("detached"));
}

#[test]
fn build_live_subagent_info_list_omits_dead_detached_and_effectively_exited_tombstones() {
    use crate::domain::session::SubagentLiveness;
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut live = SubagentEntry::new("/tmp/live.sock".into(), 1);
        live.status = SubagentStatus::Idle;
        guard.insert("live".to_string(), live);

        let mut exited = SubagentEntry::new("/tmp/exited.sock".into(), 2);
        exited.status = SubagentStatus::Exited;
        guard.insert("exited".to_string(), exited);

        let mut dead = SubagentEntry::new("/tmp/dead.sock".into(), 3);
        dead.status = SubagentStatus::Idle;
        dead.persisted_liveness = SubagentLiveness::Dead;
        guard.insert("dead".to_string(), dead);

        let mut detached = SubagentEntry::new("/tmp/detached.sock".into(), 4);
        detached.status = SubagentStatus::Running;
        detached.persisted_liveness = SubagentLiveness::Detached;
        guard.insert("detached".to_string(), detached);
    }

    let snapshot = build_subagent_info_list(&Some(reg.clone()));
    assert_eq!(
        snapshot.len(),
        4,
        "legacy snapshot helper retains tombstones"
    );

    let live = build_live_subagent_info_list(&Some(reg));
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].agent_id, "live");
    assert_eq!(live[0].status, "idle");
    assert_eq!(live[0].liveness.as_deref(), Some("live"));
}
