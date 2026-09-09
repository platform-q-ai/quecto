use super::subagent_monitor_canonical::*;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};

#[test]
fn admission_forward_allowlists_fields_and_preserves_known_descendant() {
    let value = serde_json::json!({
        "type": "admission_state_changed",
        "agent_id": "grandchild",
        "parent_id": "child",
        "untrusted": true,
        "admission": {
            "waiting": 2, "admitted": 1, "hidden": 3, "revision": 4,
            "longestWaitSeconds": 5,
            "groups": [{"group":"provider", "cooldown":{"state":"active", "remainingSeconds":6}, "extra":true}],
            "counters": {"completed":7, "refused":8, "cancelled":9, "abandoned":10},
            "untrusted": true
        }
    });
    let encoded =
        canonical_admission_forward(&value, "child", Some("parent"), &|id| id == "grandchild")
            .unwrap();
    let forwarded: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(forwarded["agent_id"], "grandchild");
    assert_eq!(forwarded["parent_id"], "child");
    assert_eq!(forwarded["admission"]["longestWaitSeconds"], 5);
    assert_eq!(forwarded["admission"]["counters"]["abandoned"], 10);
    assert!(forwarded.get("untrusted").is_none());
    assert!(forwarded["admission"].get("untrusted").is_none());
}

#[test]
fn admission_forward_falls_back_to_immediate_child_and_rejects_other_events() {
    let spoofed =
        serde_json::json!({"type":"admission_state_changed", "agent_id":"parent", "admission":{}});
    let encoded =
        canonical_admission_forward(&spoofed, "child", Some("parent"), &|_| true).unwrap();
    let forwarded: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(forwarded["agent_id"], "child");
    assert_eq!(forwarded["parent_id"], "parent");
    assert!(
        canonical_admission_forward(&serde_json::json!({"type":"other"}), "child", None, &|_| {
            true
        })
        .is_none()
    );
    assert!(
        canonical_admission_forward(
            &serde_json::json!({"type":"admission_state_changed"}),
            "child",
            None,
            &|_| true
        )
        .is_none()
    );
}

#[test]
fn line_wrappers_forward_canonical_events_and_reject_malformed_input() {
    let workflow = forward_child_workflow_event(
        r#"{"type":"workflow_state","mode":"active"}"#,
        "child",
        Some("parent"),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&workflow).unwrap()["agent_id"],
        "child"
    );

    let messages = forward_child_messages_appended(
        r#"{"type":"subagent_messages_appended","messages":[{"text":"hello"}]}"#,
        "child",
        Some("parent"),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&messages).unwrap()["messages"][0]["text"],
        "hello"
    );

    assert!(forward_child_workflow_event("not json", "child", None).is_none());
    assert!(forward_child_messages_appended("{}", "child", None).is_none());
    assert_eq!(
        bounded_forward(Some("{}".into()), "child", "test"),
        Some("{}\n".into())
    );
}

#[test]
fn admission_line_requires_exact_type_and_known_registry_descendant() {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        let mut entry = SubagentEntry::new("socket".into(), 42);
        entry.parent_id = Some("child".into());
        entries.insert("grandchild".into(), entry);
    }
    assert!(forward_child_admission_line("{}", "child", Some("parent"), &registry).is_none());
    let line =
        r#"{"type":"admission_state_changed","agent_id":"grandchild","admission":{"waiting":1}}"#;
    let forwarded = forward_child_admission_line(line, "child", Some("parent"), &registry).unwrap();
    assert!(forwarded.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(forwarded.trim()).unwrap();
    assert_eq!(value["agent_id"], "grandchild");
}
