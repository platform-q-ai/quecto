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
