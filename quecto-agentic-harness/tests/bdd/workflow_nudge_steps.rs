//! BDD steps for `workflow_auto_continue_nudge.feature`: wording of the
//! auto-continue nudge injected at idle boundaries for workflow-bound agents.
//!
//! Literal instruction-following models treated the old "Respond with just
//! the word DONE" sentence as a status poll with a mandated one-word answer,
//! replying without tool calls — which the no-progress detector then read as
//! a stall and silently disabled auto-continue for the rest of the run.
//!
//! The exact prompt fragments asserted here are an implementation detail, so
//! they live in these step definitions rather than in the feature file: the
//! Gherkin states the behaviour (no mandated status reply; error-path
//! recovery instruction) and this module owns the fragment knowledge.

use super::*;
use quecto::domain::workflow::{WorkflowConfig, WorkflowEngine};

#[given("an active workflow with incomplete steps and auto-continue enabled")]
fn given_active_workflow_with_auto_continue(world: &mut QuectoWorld) {
    let config = WorkflowConfig {
        auto_continue: true,
        ..WorkflowConfig::default()
    };
    let mut engine = WorkflowEngine::new(config, false).expect("engine builds");
    engine
        .select_template("feature", None)
        .expect("feature template selects");
    world.workflow_nudge_engine = Some(engine);
}

#[when("I request the auto-continue nudge")]
fn when_request_auto_continue_nudge(world: &mut QuectoWorld) {
    let engine = world
        .workflow_nudge_engine
        .as_ref()
        .expect("workflow engine not set");
    world.workflow_nudge_text = Some(
        engine
            .auto_continue_nudge()
            .expect("active incomplete workflow yields a nudge"),
    );
}

fn nudge_text(world: &QuectoWorld) -> &str {
    world
        .workflow_nudge_text
        .as_deref()
        .expect("auto-continue nudge not captured")
}

#[then("the nudge should not mandate a status-only reply")]
fn then_nudge_does_not_mandate_status_reply(world: &mut QuectoWorld) {
    let text = nudge_text(world);
    assert!(
        !text.contains("DONE"),
        "nudge must not mandate a one-word DONE reply: {text}"
    );
    assert!(
        !text.contains("Respond with just the word"),
        "nudge must not mandate a status-only reply: {text}"
    );
}

#[when("I request the corrective nudge")]
fn when_request_corrective_nudge(world: &mut QuectoWorld) {
    let engine = world
        .workflow_nudge_engine
        .as_ref()
        .expect("workflow engine not set");
    world.workflow_nudge_text = Some(
        engine
            .corrective_nudge()
            .expect("active incomplete workflow yields a corrective nudge"),
    );
}

#[then("the corrective nudge should demand a check-off or continued work")]
fn then_corrective_nudge_demands_check_off_or_work(world: &mut QuectoWorld) {
    let text = nudge_text(world);
    assert!(
        text.contains("did not advance the workflow"),
        "corrective nudge must name the stall: {text}"
    );
    assert!(
        text.contains("check it off with the workflow tool"),
        "corrective nudge must point at the workflow tool for the check-off: {text}"
    );
    assert!(
        text.contains("Do not reply with only a status message"),
        "corrective nudge must forbid a bare status reply: {text}"
    );
}

// ─── Cache-safe prompting (#1113): nudges carry the workflow state that no
// longer lives in the system prompt ──────────────────────────────────────────

/// A `--workflow` session before template selection: the engine sits in
/// selector mode with the idle-boundary selector nudge armed. Distinctive
/// (non-dictionary) template ids keep the listing assertions falsifiable.
fn selector_mode_workflow(world: &mut QuectoWorld, auto_continue: bool) {
    use quecto::domain::workflow::{WorkflowTemplate, WorkflowTemplateStep};
    let templates = vec![WorkflowTemplate {
        id: "qx-selector-probe".into(),
        label: "QX Selector Probe".into(),
        description: "probe template for selector nudges".into(),
        when_to_use: None,
        steps: vec![WorkflowTemplateStep {
            key: "only".into(),
            label: "Only probe step".into(),
            phase: "red".into(),
            guidance: None,
        }],
        guards: vec![],
    }];
    let config = WorkflowConfig {
        auto_continue,
        templates,
        ..WorkflowConfig::default()
    };
    let mut engine = WorkflowEngine::new(config, false).expect("engine builds");
    engine.set_selector_nudge(true);
    world.workflow_nudge_engine = Some(engine);
}

#[given("a workflow awaiting template selection with auto-continue enabled")]
fn given_selector_mode_workflow_with_auto_continue(world: &mut QuectoWorld) {
    selector_mode_workflow(world, true);
}

/// #1113 AC3 regression: the selector nudge is the sole proactive selection
/// channel and must not be gated on `workflow.auto_continue` — the retired
/// system-prompt selector reached the model regardless of that setting.
#[given("a workflow awaiting template selection with auto-continue disabled")]
fn given_selector_mode_workflow_without_auto_continue(world: &mut QuectoWorld) {
    selector_mode_workflow(world, false);
}

#[then("the nudge should carry the current step label and guidance")]
fn then_nudge_carries_current_step_label_and_guidance(world: &mut QuectoWorld) {
    let engine = world
        .workflow_nudge_engine
        .as_ref()
        .expect("workflow engine not set");
    let step = engine
        .current_step()
        .expect("incomplete workflow has a current step");
    let guidance = step
        .guidance
        .as_deref()
        .expect("first feature step carries guidance");
    let text = nudge_text(world);
    assert!(
        text.contains(&step.label),
        "nudge must carry the current step label '{}': {text}",
        step.label
    );
    assert!(
        text.contains(guidance),
        "nudge must carry the current step guidance '{guidance}': {text}"
    );
}

#[then("the nudge should present the workflow template selector")]
fn then_nudge_presents_template_selector(world: &mut QuectoWorld) {
    let text = nudge_text(world).to_string();
    let engine = world
        .workflow_nudge_engine
        .as_ref()
        .expect("workflow engine not set");
    assert!(
        text.contains("select_template"),
        "selector nudge must tell the model to select a template via select_template: {text}"
    );
    // Derive the expected listing from the engine itself so a nudge that
    // drops the template menu cannot pass on incidental prose.
    let templates = engine.list_templates();
    assert!(!templates.is_empty(), "engine must expose templates");
    for template in templates {
        assert!(
            text.contains(&template.id) && text.contains(&template.label),
            "selector nudge must list template '{}' ({}): {text}",
            template.id,
            template.label
        );
    }
}

#[then("the nudge should instruct the model how to recover from a failed tool call")]
fn then_nudge_carries_error_path_instruction(world: &mut QuectoWorld) {
    let text = nudge_text(world);
    assert!(
        text.contains("If a tool call failed, retry or work around it"),
        "nudge must tell the model to retry or work around a failed tool call: {text}"
    );
    assert!(
        text.contains("state which step is blocked and why"),
        "nudge must tell a genuinely blocked model to name the blocked step: {text}"
    );
}
