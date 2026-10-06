use super::*;

/// Given either prompt builder, the playbook belongs only to the parent.
#[test]
fn parent_playbook_is_excluded_from_children() {
    for parent in [
        build_system_prompt(&None, false),
        build_agent_system_prompt(None, None, false, ""),
    ] {
        assert!(parent.contains(parent_coordination_policy()));
    }
    for child in [
        build_system_prompt(&None, true),
        build_agent_system_prompt(None, None, true, ""),
    ] {
        assert!(child.starts_with(child_role_preamble()));
        assert!(!child.contains(agent_role_preamble()));
        assert!(!child.contains(parent_coordination_policy()));
    }
}

#[test]
fn parent_playbook_is_an_external_document_loaded_only_for_parent() {
    let document = include_str!("../../../PARENT_PLAYBOOK.md").trim_end();
    assert_eq!(parent_coordination_policy(), document);
    assert!(build_system_prompt(&None, false).contains(document));
    assert!(!build_system_prompt(&None, true).contains(document));
}

#[test]
fn selected_playbook_replaces_default_only_for_parent() {
    let marker = "CUSTOM_PARENT_PLAYBOOK_ONLY";
    let parent =
        build_agent_system_prompt_with_playbook(None, None, PromptRole::Parent, "", marker);
    assert!(parent.contains(marker));
    assert!(!parent.contains(parent_coordination_policy()));
    let child =
        build_agent_system_prompt_with_playbook(None, None, PromptRole::Subagent, "", marker);
    assert!(!child.contains(marker));
    assert!(!child.contains(parent_coordination_policy()));
}

/// #2461: a swarm coordinator starts from its own preamble and playbook,
/// never the parent's or a worker's.
#[test]
fn coordinator_gets_only_the_coordinator_playbook() {
    let document = include_str!("../../../COORDINATOR_PLAYBOOK.md").trim_end();
    let marker = "CUSTOM_PARENT_PLAYBOOK_ONLY";
    let coordinator =
        build_agent_system_prompt_with_playbook(None, None, PromptRole::Coordinator, "", marker);
    assert!(coordinator.starts_with(coordinator_role_preamble()));
    assert!(coordinator.contains(document));
    assert!(!coordinator.contains(marker));
    assert!(!coordinator.contains(parent_coordination_policy()));
    assert!(!coordinator.contains(agent_role_preamble()));
    assert!(!coordinator.contains(child_role_preamble()));
}

/// #2461: the coordinator playbook reaches no other role.
#[test]
fn coordinator_playbook_is_excluded_from_parent_and_subagents() {
    let document = include_str!("../../../COORDINATOR_PLAYBOOK.md").trim_end();
    for role in [PromptRole::Parent, PromptRole::Subagent] {
        let prompt = build_agent_system_prompt_with_playbook(None, None, role, "", "PLAYBOOK");
        assert!(!prompt.contains(document), "{role:?}");
        assert!(!prompt.contains(coordinator_role_preamble()), "{role:?}");
    }
}

/// #2461: the role comes only from the explicit launch flags.
#[test]
fn prompt_role_follows_the_launch_flags() {
    assert_eq!(PromptRole::of(false, false), PromptRole::Parent);
    assert_eq!(PromptRole::of(true, false), PromptRole::Subagent);
    assert_eq!(PromptRole::of(true, true), PromptRole::Coordinator);
}
