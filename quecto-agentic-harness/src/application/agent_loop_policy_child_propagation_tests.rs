use super::tests::*;
use crate::domain::tool::{
    ChildToolPolicyPropagation, ChildToolPolicyPropagationStatus, ToolPolicyApplyMode,
    ToolPolicyChildPropagator, ToolPolicyMutation,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingChildPropagator {
    calls: Mutex<Vec<(Vec<String>, ToolPolicyApplyMode)>>,
}

impl RecordingChildPropagator {
    fn calls(&self) -> Vec<(Vec<String>, ToolPolicyApplyMode)> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ToolPolicyChildPropagator for RecordingChildPropagator {
    fn has_children(&self) -> bool {
        true
    }

    fn propagate_tool_policy_to_children(
        &self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> Vec<ChildToolPolicyPropagation> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((
                mutations
                    .iter()
                    .map(|mutation| mutation.name.clone())
                    .collect(),
                mode,
            ));
        vec![ChildToolPolicyPropagation {
            agent_id: "child-1".to_string(),
            status: ChildToolPolicyPropagationStatus::Queued,
            reconciliation: None,
            error: None,
        }]
    }
}

#[test]
fn queued_policy_propagates_to_existing_children_at_boundary() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let propagator = Arc::new(RecordingChildPropagator::default());
    agent.tool_policy_child_propagator = Some(propagator.clone());

    assert!(
        agent
            .request_tool_policy_mutation(
                &[ToolPolicyMutation::disable("alpha", "next boundary")],
                ToolPolicyApplyMode::AtNextTurnBoundary,
            )
            .is_none(),
        "boundary requests are queued on the parent"
    );
    assert!(
        propagator.calls().is_empty(),
        "children are not updated before the safe boundary"
    );

    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued policy mutations drain at the boundary");

    assert_eq!(
        propagator.calls(),
        vec![(
            vec!["alpha".to_string()],
            ToolPolicyApplyMode::AtNextTurnBoundary
        )]
    );
    assert_eq!(reconciliation.child_propagation.len(), 1);
    assert_eq!(
        reconciliation.child_propagation[0].status,
        ChildToolPolicyPropagationStatus::Queued
    );
}

#[test]
fn immediate_if_idle_with_existing_children_applies_and_propagates_when_parent_idle() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let propagator = Arc::new(RecordingChildPropagator::default());
    agent.tool_policy_child_propagator = Some(propagator.clone());

    let reconciliation = agent
        .request_tool_policy_mutation(
            &[ToolPolicyMutation::disable("alpha", "tui modal")],
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .expect("idle parent ImmediateIfIdle applies immediately");

    assert_eq!(
        propagator.calls(),
        vec![(
            vec!["alpha".to_string()],
            ToolPolicyApplyMode::ImmediateIfIdle
        )]
    );
    assert!(agent.drain_tool_policy_mutations_at_boundary().is_none());
    assert_eq!(reconciliation.child_propagation.len(), 1);
}

#[test]
fn in_flight_immediate_policy_propagates_to_children_when_boundary_applies() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let propagator = Arc::new(RecordingChildPropagator::default());
    agent.tool_policy_child_propagator = Some(propagator.clone());
    agent.mark_turn_in_flight();

    assert!(
        agent
            .request_tool_policy_mutation(
                &[ToolPolicyMutation::disable("alpha", "while busy")],
                ToolPolicyApplyMode::ImmediateIfIdle,
            )
            .is_none(),
        "immediateIfIdle queues while the parent turn is in flight"
    );

    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("busy-time policy mutations drain at turn boundary");

    assert_eq!(
        propagator.calls(),
        vec![(
            vec!["alpha".to_string()],
            ToolPolicyApplyMode::AtNextTurnBoundary
        )]
    );
    assert_eq!(reconciliation.child_propagation.len(), 1);
}
