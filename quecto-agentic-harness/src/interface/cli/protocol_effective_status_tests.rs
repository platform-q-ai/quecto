#[test]
fn build_subagent_info_list_reports_idle_parent_with_running_direct_child_as_running() {
    use super::build_subagent_info_list;
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut parent = SubagentEntry::new("/tmp/parent.sock".into(), 1);
        parent.status = SubagentStatus::Idle;
        guard.insert("parent".to_string(), parent);
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 2);
        child.status = SubagentStatus::Running;
        child.parent_id = Some("parent".to_string());
        guard.insert("child".to_string(), child);
    }
    let list = build_subagent_info_list(&Some(reg));
    let parent = list.iter().find(|info| info.agent_id == "parent").unwrap();
    assert_eq!(parent.status, "running");
}

#[test]
fn parent_identity_from_session_key_covers_generated_named_colon_raw_and_empty() {
    use crate::infrastructure::tools::subagent_identity::parent_identity_from_session_key;

    assert_eq!(
        parent_identity_from_session_key("chat-abc"),
        Some("chat-abc")
    );
    assert_eq!(parent_identity_from_session_key("cli:named"), Some("named"));
    assert_eq!(
        parent_identity_from_session_key("uds:resumed"),
        Some("resumed")
    );
    assert_eq!(parent_identity_from_session_key("raw"), Some("raw"));
    assert_eq!(parent_identity_from_session_key(""), None);
}

#[test]
fn effective_descendant_activity_reports_false_for_missing_registry_or_no_children() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, has_active_descendant_for_agent, new_registry,
    };

    assert!(!has_active_descendant_for_agent(&None, "parent"));

    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut unrelated = SubagentEntry::new("/tmp/unrelated.sock".into(), 1);
        unrelated.status = SubagentStatus::Running;
        unrelated.parent_id = Some("other".to_string());
        guard.insert("unrelated".to_string(), unrelated);
    }

    assert!(!has_active_descendant_for_agent(&Some(reg), "parent"));
}

#[test]
fn build_subagent_info_list_preserves_own_starting_status() {
    use super::build_subagent_info_list;
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut entry = SubagentEntry::new("/tmp/starting.sock".into(), 1);
        entry.status = SubagentStatus::Starting;
        guard.insert("starting".to_string(), entry);
    }
    let list = build_subagent_info_list(&Some(reg));
    let info = list
        .iter()
        .find(|info| info.agent_id == "starting")
        .unwrap();
    assert_eq!(info.status, "starting");
}

#[test]
fn build_subagent_info_list_does_not_roll_up_ambiguous_display_name_parent_id() {
    use super::build_subagent_info_list;
    use crate::domain::ids::AgentUuid;
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let first_uuid = AgentUuid::mint();
        let mut first = SubagentEntry::with_identity(
            first_uuid.clone(),
            "dup".to_string(),
            "/tmp/first.sock".into(),
            1,
        );
        first.status = SubagentStatus::Idle;
        guard.insert(first_uuid.to_string(), first);
        let second_uuid = AgentUuid::mint();
        let mut second = SubagentEntry::with_identity(
            second_uuid.clone(),
            "dup".to_string(),
            "/tmp/second.sock".into(),
            2,
        );
        second.status = SubagentStatus::Idle;
        guard.insert(second_uuid.to_string(), second);
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 3);
        child.status = SubagentStatus::Running;
        child.parent_id = Some("dup".to_string());
        guard.insert("child".to_string(), child);
    }
    let list = build_subagent_info_list(&Some(reg));
    let duplicate_parents: Vec<_> = list.iter().filter(|info| info.agent_id == "dup").collect();
    assert_eq!(duplicate_parents.len(), 2);
    assert!(duplicate_parents.iter().all(|info| info.status == "idle"));
}

#[test]
fn build_subagent_info_list_rolls_up_child_using_parent_display_name_when_parent_key_is_uuid() {
    use super::build_subagent_info_list;
    use crate::domain::ids::AgentUuid;
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let parent_uuid = AgentUuid::mint();
        let mut parent = SubagentEntry::with_identity(
            parent_uuid.clone(),
            "parent".to_string(),
            "/tmp/parent.sock".into(),
            1,
        );
        parent.status = SubagentStatus::Idle;
        guard.insert(parent_uuid.to_string(), parent);
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 2);
        child.status = SubagentStatus::Running;
        child.parent_id = Some("parent".to_string());
        guard.insert("child".to_string(), child);
    }
    let list = build_subagent_info_list(&Some(reg));
    let parent = list.iter().find(|info| info.agent_id == "parent").unwrap();
    assert_eq!(parent.status, "running");
}

#[test]
fn build_subagent_info_list_reports_idle_parent_with_running_grandchild_as_running() {
    use super::build_subagent_info_list;
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut parent = SubagentEntry::new("/tmp/parent.sock".into(), 1);
        parent.status = SubagentStatus::Idle;
        guard.insert("parent".to_string(), parent);
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 2);
        child.status = SubagentStatus::Idle;
        child.parent_id = Some("parent".to_string());
        guard.insert("child".to_string(), child);
        let mut grandchild = SubagentEntry::new("/tmp/grandchild.sock".into(), 3);
        grandchild.status = SubagentStatus::Starting;
        grandchild.parent_id = Some("child".to_string());
        guard.insert("grandchild".to_string(), grandchild);
    }
    let list = build_subagent_info_list(&Some(reg));
    let parent = list.iter().find(|info| info.agent_id == "parent").unwrap();
    assert_eq!(parent.status, "running");
}
