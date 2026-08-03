use super::tests::*;
use crate::domain::tool::{
    ChildToolPolicyPropagation, ChildToolPolicyPropagationStatus, ToolPolicyApplyMode,
    ToolPolicyChildPropagator, ToolPolicyMutation,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingChildPropagator {
    calls: Mutex<Vec<(Vec<String>, ToolPolicyApplyMode)>>,
    statuses: Mutex<Vec<ChildToolPolicyPropagationStatus>>,
}

impl RecordingChildPropagator {
    fn with_statuses(statuses: Vec<ChildToolPolicyPropagationStatus>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            statuses: Mutex::new(statuses),
        }
    }

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
        let status = self
            .statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
            .unwrap_or(ChildToolPolicyPropagationStatus::Timeout);
        let error =
            (status != ChildToolPolicyPropagationStatus::Queued).then(|| format!("{status:?}"));
        vec![ChildToolPolicyPropagation {
            agent_id: "child-1".to_string(),
            status,
            reconciliation: None,
            error,
        }]
    }
}

struct MultiChildPropagator {
    statuses: Vec<ChildToolPolicyPropagationStatus>,
}

impl ToolPolicyChildPropagator for MultiChildPropagator {
    fn has_children(&self) -> bool {
        true
    }

    fn propagate_tool_policy_to_children(
        &self,
        _mutations: &[ToolPolicyMutation],
        _mode: ToolPolicyApplyMode,
    ) -> Vec<ChildToolPolicyPropagation> {
        self.statuses
            .iter()
            .enumerate()
            .map(|(index, status)| ChildToolPolicyPropagation {
                agent_id: format!("child-{index}"),
                status: *status,
                reconciliation: None,
                error: (*status != ChildToolPolicyPropagationStatus::Queued)
                    .then(|| format!("{status:?}")),
            })
            .collect()
    }
}

#[test]
fn queued_response_child_results_are_absent_without_prompt_child_propagation() {
    let (agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    assert!(
        agent
            .recent_tool_policy_child_propagation_for_response()
            .is_none()
    );
}

#[test]
fn one_busy_parent_update_response_includes_all_child_results() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let propagator = Arc::new(MultiChildPropagator {
        statuses: vec![
            ChildToolPolicyPropagationStatus::Timeout,
            ChildToolPolicyPropagationStatus::Queued,
        ],
    });
    agent.tool_policy_child_propagator = Some(propagator);
    agent.mark_turn_in_flight();

    assert!(
        agent
            .request_tool_policy_mutation(
                &[ToolPolicyMutation::disable("alpha", "busy policy")],
                ToolPolicyApplyMode::ImmediateIfIdle,
            )
            .is_none()
    );

    let response_results = agent
        .recent_tool_policy_child_propagation_for_response()
        .expect("queued response surfaces child propagation");
    let statuses: Vec<_> = response_results
        .iter()
        .map(|result| result.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            ChildToolPolicyPropagationStatus::Timeout,
            ChildToolPolicyPropagationStatus::Queued,
        ],
        "one mutation can fan out to multiple child propagation outcomes"
    );
}

#[test]
fn multiple_busy_parent_updates_preserve_all_prompt_child_results_until_boundary() {
    let (mut agent, _provider) = make_agent(
        vec![text_response("done")],
        vec![("alpha", "ok"), ("beta", "ok")],
    );
    let propagator = Arc::new(RecordingChildPropagator::with_statuses(vec![
        ChildToolPolicyPropagationStatus::Queued,
        ChildToolPolicyPropagationStatus::Timeout,
    ]));
    agent.tool_policy_child_propagator = Some(propagator.clone());
    agent.mark_turn_in_flight();

    assert!(
        agent
            .request_tool_policy_mutation(
                &[ToolPolicyMutation::disable("alpha", "first busy policy")],
                ToolPolicyApplyMode::ImmediateIfIdle,
            )
            .is_none()
    );
    let first_response_results = agent
        .recent_tool_policy_child_propagation_for_response()
        .expect("first queued response surfaces first child failure");
    assert_eq!(first_response_results.len(), 1);
    assert_eq!(
        first_response_results[0].status,
        ChildToolPolicyPropagationStatus::Timeout
    );

    assert!(
        agent
            .request_tool_policy_mutation(
                &[ToolPolicyMutation::disable("beta", "second busy policy")],
                ToolPolicyApplyMode::ImmediateIfIdle,
            )
            .is_none()
    );
    let second_response_results = agent
        .recent_tool_policy_child_propagation_for_response()
        .expect("second queued response surfaces child result for that request");
    assert_eq!(second_response_results.len(), 1);
    assert_eq!(
        second_response_results[0].status,
        ChildToolPolicyPropagationStatus::Queued
    );

    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("combined parent boundary drain");

    assert_eq!(
        propagator.calls().len(),
        2,
        "boundary must not propagate a third time"
    );
    let statuses: Vec<_> = reconciliation
        .child_propagation
        .iter()
        .map(|result| result.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            ChildToolPolicyPropagationStatus::Timeout,
            ChildToolPolicyPropagationStatus::Queued,
        ],
        "boundary event must retain earlier failure and later queued result"
    );
}

#[test]
fn idle_boundary_policy_propagates_to_existing_children_immediately() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let propagator = Arc::new(RecordingChildPropagator::default());
    agent.tool_policy_child_propagator = Some(propagator.clone());

    let reconciliation = agent
        .request_tool_policy_mutation(
            &[ToolPolicyMutation::disable("alpha", "next boundary")],
            ToolPolicyApplyMode::AtNextTurnBoundary,
        )
        .expect("idle boundary request applies before the next provider request");

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
        ChildToolPolicyPropagationStatus::Timeout
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
fn in_flight_immediate_policy_propagates_to_children_promptly_and_not_again_at_parent_boundary() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let propagator = Arc::new(RecordingChildPropagator::default());
    agent.tool_policy_child_propagator = Some(propagator.clone());
    agent.mark_turn_in_flight();

    let immediate = agent.request_tool_policy_mutation(
        &[ToolPolicyMutation::disable("alpha", "while busy")],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert!(
        immediate.is_none(),
        "immediateIfIdle queues local parent application while the parent turn is in flight"
    );
    let prompt_child_results = agent
        .recent_tool_policy_child_propagation_for_response()
        .expect("busy-parent response must preserve prompt child propagation failures");
    assert_eq!(prompt_child_results.len(), 1);
    assert_eq!(
        prompt_child_results[0].status,
        ChildToolPolicyPropagationStatus::Timeout
    );

    assert_eq!(
        propagator.calls(),
        vec![(
            vec!["alpha".to_string()],
            ToolPolicyApplyMode::AtNextTurnBoundary
        )],
        "busy parent must promptly forward boundary intent to existing children"
    );

    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("busy-time policy mutations drain at turn boundary");

    assert_eq!(
        propagator.calls().len(),
        1,
        "parent boundary drain must not duplicate an already-forwarded child propagation"
    );
    assert_eq!(reconciliation.child_propagation.len(), 1);
    assert_eq!(
        reconciliation.child_propagation[0].status,
        ChildToolPolicyPropagationStatus::Timeout
    );
}
