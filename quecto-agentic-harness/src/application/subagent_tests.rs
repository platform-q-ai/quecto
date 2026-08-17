use super::*;

#[test]
fn test_subagent_context_has_empty_history() {
    let config = SubagentConfig {
        container: crate::domain::subagent::ContainerSelection::Local,
        task: Some("Do stuff".to_string()),
        agent_id: None,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    let ctx = SubagentContext::from_config(&config);
    assert_eq!(ctx.task, "Do stuff");
    assert!(ctx.messages.is_empty());
}

#[test]
fn test_subagent_context_no_task() {
    let config = SubagentConfig {
        container: crate::domain::subagent::ContainerSelection::Local,
        task: None,
        agent_id: None,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    };
    let ctx = SubagentContext::from_config(&config);
    assert_eq!(ctx.task, "");
    assert!(ctx.messages.is_empty());
}
