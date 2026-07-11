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
