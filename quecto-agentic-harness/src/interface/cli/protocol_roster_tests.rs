use super::*;
use crate::domain::session::SubagentLiveness;
use crate::infrastructure::tools::subagent_monitor::update_entry_next_sequence;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};

fn row_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn compact_roster_rows_have_exact_shape_and_no_rich_leaks() {
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut entry = SubagentEntry::new("/tmp/worker.sock".into(), 4321);
        entry.status = SubagentStatus::Running;
        entry.last_tool = Some("bash".into());
        entry.last_error = Some("boom".into());
        entry.parent_id = Some("parent".into());
        entry.read_only = true;
        entry.environment_ref = Some("C1".into());
        entry.notification_sequence = 1;
        guard.insert("worker".into(), entry);
    }

    let rows = build_compact_subagent_roster(&Some(reg), None).unwrap();
    assert_eq!(rows.sequence, 1);
    let value = serde_json::to_value(&rows.subagents[0]).unwrap();
    assert_eq!(
        row_keys(&value),
        vec!["agentId", "environmentRef", "status"]
    );
    for forbidden in [
        "environment",
        "agentUuid",
        "displayName",
        "lastTool",
        "lastError",
        "liveness",
        "pid",
        "socketPath",
        "parentId",
        "workflow",
        "readOnly",
        "executionBackend",
    ] {
        assert!(
            value.get(forbidden).is_none(),
            "forbidden field leaked: {forbidden}"
        );
    }
}

#[test]
fn compact_roster_maps_status_and_liveness_to_four_words() {
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        for (id, status, liveness, expected) in [
            (
                "a",
                SubagentStatus::Starting,
                SubagentLiveness::Live,
                "running",
            ),
            (
                "b",
                SubagentStatus::Running,
                SubagentLiveness::Live,
                "running",
            ),
            ("c", SubagentStatus::Idle, SubagentLiveness::Live, "idle"),
            (
                "d",
                SubagentStatus::Error,
                SubagentLiveness::Live,
                "errored",
            ),
            ("e", SubagentStatus::Exited, SubagentLiveness::Live, "dead"),
            (
                "f",
                SubagentStatus::Idle,
                SubagentLiveness::Detached,
                "dead",
            ),
            ("g", SubagentStatus::Running, SubagentLiveness::Dead, "dead"),
        ] {
            let mut entry = SubagentEntry::new(format!("/tmp/{id}.sock").into(), 1);
            entry.status = status;
            entry.persisted_liveness = liveness;
            guard.insert(format!("{id}:{expected}"), entry);
        }
    }
    let rows = build_compact_subagent_roster(&Some(reg), None).unwrap();
    for row in rows.subagents {
        let expected = row.agent_id.split(':').nth(1).unwrap();
        assert_eq!(
            row.status, expected,
            "wrong compact status for {}",
            row.agent_id
        );
    }
}

#[test]
fn compact_roster_since_current_is_unchanged_and_future_is_rejected() {
    let reg = new_registry();
    reg.lock()
        .unwrap()
        .insert("a".into(), SubagentEntry::new("/tmp/a.sock".into(), 1));
    let full = build_compact_subagent_roster(&Some(reg.clone()), None).unwrap();
    let unchanged = build_compact_subagent_roster(&Some(reg.clone()), Some(full.sequence)).unwrap();
    assert_eq!(unchanged.sequence, full.sequence);
    assert_eq!(unchanged.unchanged, Some(true));
    assert!(unchanged.subagents.is_empty());
    assert!(build_compact_subagent_roster(&Some(reg), Some(full.sequence + 1)).is_err());
}

#[test]
fn registry_updates_allocate_roster_global_sequence() {
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        guard.insert("a".into(), SubagentEntry::new("/tmp/a.sock".into(), 1));
        guard.insert("b".into(), SubagentEntry::new("/tmp/b.sock".into(), 2));
    }
    assert_eq!(
        update_entry_next_sequence(&reg, "a", |e| e.status = SubagentStatus::Idle),
        1
    );
    let full = build_compact_subagent_roster(&Some(reg.clone()), None).unwrap();
    assert_eq!(full.sequence, 1);
    assert_eq!(
        update_entry_next_sequence(&reg, "b", |e| e.status = SubagentStatus::Error),
        2
    );
    let delta = build_compact_subagent_roster(&Some(reg), Some(full.sequence)).unwrap();
    assert_eq!(delta.sequence, 2);
    assert_eq!(delta.subagents.len(), 1);
    assert_eq!(delta.subagents[0].agent_id, "b");
}

#[test]
fn get_subagents_command_round_trips_since_cursor() {
    let cmd = AgentCommand::GetSubagents {
        id: Some("gs".into()),
        since: Some(42),
    };
    let value = serde_json::to_value(&cmd).unwrap();
    assert_eq!(value["type"], "get_subagents");
    assert_eq!(value["since"], 42);
    let parsed: AgentCommand = serde_json::from_value(value).unwrap();
    match parsed {
        AgentCommand::GetSubagents { id, since } => {
            assert_eq!(id.as_deref(), Some("gs"));
            assert_eq!(since, Some(42));
        }
        _ => panic!("expected get_subagents"),
    }
}

#[test]
fn compact_roster_since_old_returns_changed_rows_only() {
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut unchanged = SubagentEntry::new("/tmp/a.sock".into(), 1);
        unchanged.notification_sequence = 1;
        guard.insert("a".into(), unchanged);
        let mut changed = SubagentEntry::new("/tmp/b.sock".into(), 2);
        changed.notification_sequence = 3;
        changed.status = SubagentStatus::Idle;
        guard.insert("b".into(), changed);
    }
    let delta = build_compact_subagent_roster(&Some(reg), Some(1)).unwrap();
    assert_eq!(delta.sequence, 3);
    assert_eq!(delta.subagents.len(), 1);
    assert_eq!(delta.subagents[0].agent_id, "b");
}
