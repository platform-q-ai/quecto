use super::*;

#[test]
fn parent_prompt_contains_only_role_and_routing_guidance() {
    let result = build_system_prompt(&None, false);
    assert!(!result.contains("Current date and time:"));
    assert!(result.contains(agent_role_preamble()));
    assert!(result.contains("Parent Agent"));
    for excluded in [
        "`docs` tool",
        "operating manual",
        "quick-start",
        "definitive source",
        "name `quecto`",
        "quecto-tui",
        "quecto-api",
        "quecto-mcp",
    ] {
        assert!(!result.contains(excluded));
    }
}

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
