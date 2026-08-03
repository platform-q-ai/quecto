use quecto::domain::tool::{
    ChildToolPolicyPropagation, ChildToolPolicyPropagationStatus, ToolPolicyApplyMode,
    ToolPolicyChildPropagator, ToolPolicyMutation,
};
use std::sync::Mutex;

#[derive(Default)]
struct RecordingPropagator {
    calls: Mutex<Vec<(Vec<String>, ToolPolicyApplyMode)>>,
}

impl ToolPolicyChildPropagator for RecordingPropagator {
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
            agent_id: "child-1".into(),
            status: ChildToolPolicyPropagationStatus::Queued,
            reconciliation: None,
            error: None,
        }]
    }
}

#[test]
fn propagator_receives_mutations_and_mode_and_returns_child_results() {
    let propagator = RecordingPropagator::default();
    let result = propagator.propagate_tool_policy_to_children(
        &[ToolPolicyMutation::disable("bash", "policy update")],
        ToolPolicyApplyMode::AtNextTurnBoundary,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].agent_id, "child-1");
    assert_eq!(result[0].status, ChildToolPolicyPropagationStatus::Queued);
    assert_eq!(
        propagator
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_slice(),
        &[(
            vec!["bash".to_string()],
            ToolPolicyApplyMode::AtNextTurnBoundary
        )]
    );
}
