use super::agent_loop::AgentLoopImpl;
use crate::domain::tool::{ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyReconciliation};
use std::sync::atomic::Ordering;

impl AgentLoopImpl {
    pub(super) fn mark_turn_in_flight(&self) {
        self.turn_in_flight.store(true, Ordering::SeqCst);
    }

    pub fn request_tool_policy_mutation(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> Option<ToolPolicyReconciliation> {
        if mode == ToolPolicyApplyMode::ImmediateIfIdle
            && self.turn_in_flight.load(Ordering::SeqCst)
        {
            self.queue_tool_policy_mutation(mutations);
            return None;
        }
        Some(
            self.tool_registry
                .apply_tool_policy_mutations(mutations, mode),
        )
    }

    pub fn queue_tool_policy_mutation(&self, mutations: &[ToolPolicyMutation]) {
        let mut pending = self
            .pending_tool_policy_mutations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.extend_from_slice(mutations);
    }

    pub fn drain_tool_policy_mutations_at_boundary(&mut self) -> Option<ToolPolicyReconciliation> {
        let mutations = {
            let mut pending = self
                .pending_tool_policy_mutations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pending.is_empty() {
                self.turn_in_flight.store(false, Ordering::SeqCst);
                return None;
            }
            std::mem::take(&mut *pending)
        };
        self.turn_in_flight.store(false, Ordering::SeqCst);
        Some(
            self.tool_registry
                .apply_tool_policy_mutations(&mutations, ToolPolicyApplyMode::AtNextTurnBoundary),
        )
    }
}
