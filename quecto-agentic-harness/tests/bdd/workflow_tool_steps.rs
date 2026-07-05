use super::*;
use quecto::domain::workflow::{
    WorkflowConfig, WorkflowGuardRule, WorkflowTemplate, WorkflowTemplateStep,
};
use quecto::infrastructure::tools::workflow_tool::WorkflowTool;

fn guarded_template() -> WorkflowTemplate {
    WorkflowTemplate {
        id: "wave".to_string(),
        label: "Coverage Wave".to_string(),
        description: "Behavioral coverage wave".to_string(),
        when_to_use: Some("when extending BDD coverage".to_string()),
        steps: vec![
            WorkflowTemplateStep {
                key: "plan".to_string(),
                label: "Plan coverage".to_string(),
                phase: "red".to_string(),
                guidance: Some("choose behavior first".to_string()),
            },
            WorkflowTemplateStep {
                key: "tests".to_string(),
                label: "Add behavioral tests".to_string(),
                phase: "red".to_string(),
                guidance: None,
            },
            WorkflowTemplateStep {
                key: "verify".to_string(),
                label: "Verify".to_string(),
                phase: "green".to_string(),
                guidance: None,
            },
        ],
        guards: vec![WorkflowGuardRule {
            commands: vec!["cargo test".to_string()],
            before_step_key: "verify".to_string(),
            message: "finish workflow tests first".to_string(),
        }],
    }
}

fn workflow_tool(world: &QuectoWorld) -> &WorkflowTool {
    world.workflow_tool.as_ref().expect("workflow tool not set")
}

fn execute_workflow_action(world: &mut QuectoWorld, arguments: &str) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(workflow_tool(world).execute(arguments))
        .expect("workflow tool execution failed");
    world.workflow_tool_result = Some(result);
}

#[given("a workflow tool for a three-step guarded template")]
fn given_workflow_tool_for_three_step_guarded_template(world: &mut QuectoWorld) {
    let engine = quecto::domain::workflow::WorkflowEngine::new(
        WorkflowConfig {
            templates: vec![guarded_template()],
            ..WorkflowConfig::default()
        },
        true,
    )
    .expect("workflow engine should be valid");
    let events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let events_for_emitter = events.clone();
    let emitter = Arc::new(move |event: serde_json::Value| {
        events_for_emitter.lock().unwrap().push(event);
    });
    world.workflow_events = Some(events);
    world.workflow_tool = Some(WorkflowTool::with_event_emitter(
        Arc::new(Mutex::new(engine)),
        emitter,
    ));
    world.workflow_tool_result = None;
}

#[given(expr = "the workflow template {string} is selected")]
fn given_workflow_template_is_selected(world: &mut QuectoWorld, template: String) {
    execute_workflow_action(
        world,
        &format!(
            r#"{{"action":"select_template","template":"{}"}}"#,
            template
        ),
    );
    assert!(
        !world.workflow_tool_result.as_ref().unwrap().is_error,
        "template selection should succeed: {}",
        world.workflow_tool_result.as_ref().unwrap().content
    );
}

#[given("workflow events are cleared")]
fn given_workflow_events_are_cleared(world: &mut QuectoWorld) {
    world
        .workflow_events
        .as_ref()
        .expect("workflow events not set")
        .lock()
        .unwrap()
        .clear();
}

#[given(expr = "workflow step {int} is checked through the tool")]
fn given_workflow_step_is_checked_through_tool(world: &mut QuectoWorld, number: usize) {
    execute_workflow_action(world, &format!(r#"{{"action":"check","step":{}}}"#, number));
    assert!(
        !world.workflow_tool_result.as_ref().unwrap().is_error,
        "checking step should succeed: {}",
        world.workflow_tool_result.as_ref().unwrap().content
    );
}

#[when(expr = "I run workflow action {string}")]
fn when_i_run_workflow_action(world: &mut QuectoWorld, arguments: String) {
    execute_workflow_action(world, &arguments);
}

#[then("the workflow tool result should not be an error")]
fn then_workflow_tool_result_should_not_be_error(world: &mut QuectoWorld) {
    let result = world
        .workflow_tool_result
        .as_ref()
        .expect("workflow result not set");
    assert!(
        !result.is_error,
        "unexpected workflow error: {}",
        result.content
    );
}

#[then("the workflow tool result should be an error")]
fn then_workflow_tool_result_should_be_error(world: &mut QuectoWorld) {
    let result = world
        .workflow_tool_result
        .as_ref()
        .expect("workflow result not set");
    assert!(
        result.is_error,
        "expected workflow error, got: {}",
        result.content
    );
}

#[then(expr = "the workflow tool result should contain {string}")]
fn then_workflow_tool_result_should_contain(world: &mut QuectoWorld, expected: String) {
    let result = world
        .workflow_tool_result
        .as_ref()
        .expect("workflow result not set");
    assert!(
        result.content.contains(&expected),
        "expected workflow result to contain '{}', got: {}",
        expected,
        result.content
    );
}

#[then(expr = "the last workflow event should have mode {string}")]
fn then_last_workflow_event_should_have_mode(world: &mut QuectoWorld, expected: String) {
    let event = last_workflow_event(world);
    assert_eq!(event["type"].as_str(), Some("workflow_state"));
    assert_eq!(
        event["mode"].as_str(),
        Some(expected.as_str()),
        "event: {event}"
    );
}

#[then(expr = "the last workflow event should have active issue number {int} and title {string}")]
fn then_last_workflow_event_should_have_active_issue(
    world: &mut QuectoWorld,
    issue_number: usize,
    title: String,
) {
    let event = last_workflow_event(world);
    assert_eq!(
        event["activeIssue"]["number"].as_u64(),
        Some(issue_number as u64)
    );
    assert_eq!(event["activeIssue"]["title"].as_str(), Some(title.as_str()));
}

#[then(expr = "the last workflow event current step should be {int} with key {string}")]
fn then_last_workflow_event_current_step_should_be(
    world: &mut QuectoWorld,
    step_index: usize,
    key: String,
) {
    let event = last_workflow_event(world);
    assert_eq!(
        event["currentStep"]["index"].as_u64(),
        Some(step_index as u64)
    );
    assert_eq!(event["currentStep"]["key"].as_str(), Some(key.as_str()));
}

#[then("no workflow event should be emitted")]
fn then_no_workflow_event_should_be_emitted(world: &mut QuectoWorld) {
    let events = world
        .workflow_events
        .as_ref()
        .expect("workflow events not set");
    let events = events.lock().unwrap();
    assert!(
        events.is_empty(),
        "expected no workflow events, got: {events:?}"
    );
}

fn last_workflow_event(world: &QuectoWorld) -> serde_json::Value {
    world
        .workflow_events
        .as_ref()
        .expect("workflow events not set")
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("no workflow event emitted")
}
