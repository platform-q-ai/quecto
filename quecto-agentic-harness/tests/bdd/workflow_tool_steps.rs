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
                guidance: Some("write failing tests first".to_string()),
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

// ─── Cache-safe prompting (#1113): guidance travels in tool results ─────────

/// Declarative wrapper over the raw-JSON action step: the behaviour is "the
/// model selects a template", not a wire-format literal.
#[when(expr = "the model selects the workflow template {string}")]
fn when_model_selects_workflow_template(world: &mut QuectoWorld, template: String) {
    execute_workflow_action(
        world,
        &format!(r#"{{"action":"select_template","template":"{template}"}}"#),
    );
}

#[when(expr = "the model checks off workflow step {int}")]
fn when_model_checks_off_workflow_step(world: &mut QuectoWorld, number: usize) {
    execute_workflow_action(world, &format!(r#"{{"action":"check","step":{number}}}"#));
}

#[when("the model requests the workflow status")]
fn when_model_requests_workflow_status(world: &mut QuectoWorld) {
    execute_workflow_action(world, r#"{"action":"status"}"#);
}

/// #1113 cache-safe prompting: with no selector text injected into the system
/// prompt, the tool's own schema description must advertise how to discover
/// and select templates.
#[when("I read the workflow tool definition")]
fn when_read_workflow_tool_definition(world: &mut QuectoWorld) {
    world.workflow_tool_definition = Some(workflow_tool(world).definition());
}

#[then(
    "the definition description should advertise the list_templates and select_template actions"
)]
fn then_definition_description_advertises_template_selection(world: &mut QuectoWorld) {
    let definition = world
        .workflow_tool_definition
        .as_ref()
        .expect("workflow tool definition not read");
    for needle in ["list_templates", "select_template"] {
        assert!(
            definition.description.contains(needle),
            "workflow tool description must advertise template selection via '{needle}': {}",
            definition.description
        );
    }
}

/// #1113 AC2: assert against the engine's own current step — not fixture
/// magic strings — that the last tool result hands the model the step's
/// label and guidance. After a `check`, the engine's current step IS the
/// next step, so both phrasings share this assertion.
#[then("the workflow tool result should carry the current step's label and guidance")]
#[then("the workflow tool result should carry the next step's label and guidance")]
fn then_result_carries_engine_current_step(world: &mut QuectoWorld) {
    let result = world
        .workflow_tool_result
        .as_ref()
        .expect("workflow result not set");
    let engine = workflow_tool(world).engine().lock().unwrap();
    let step = engine
        .current_step()
        .expect("workflow must have an incomplete current step");
    let guidance = step
        .guidance
        .as_deref()
        .expect("fixture step must carry guidance");
    assert!(
        result.content.contains(&step.label),
        "tool result must carry the step label '{}': {}",
        step.label,
        result.content
    );
    assert!(
        result.content.contains(guidance),
        "tool result must carry the step guidance '{guidance}': {}",
        result.content
    );
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
