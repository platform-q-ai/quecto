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
    let parent = build_agent_system_prompt_with_playbook(None, None, false, "", marker);
    assert!(parent.contains(marker));
    assert!(!parent.contains(parent_coordination_policy()));
    let child = build_agent_system_prompt_with_playbook(None, None, true, "", marker);
    assert!(!child.contains(marker));
    assert!(!child.contains(parent_coordination_policy()));
}
