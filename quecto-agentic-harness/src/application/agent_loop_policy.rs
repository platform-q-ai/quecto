use super::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::tool::{ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyReconciliation};
use crate::domain::tool_descriptor::ToolCatalogueEntry;
use std::sync::atomic::Ordering;

impl AgentLoopImpl {
    pub(super) fn notify_tool_catalogue_changed(
        &self,
        changed_tools: Vec<String>,
        before: Vec<ToolCatalogueEntry>,
        reason: &str,
    ) {
        let after = self.tool_catalogue_entries();
        if before == after {
            return;
        }
        self.notify(|| AgentProgressEvent::ToolCatalogueChanged {
            changed_tools,
            before,
            after,
            reason: reason.to_string(),
        });
    }

    pub(super) fn notify_tool_policy_changed(
        &self,
        reconciliation: &ToolPolicyReconciliation,
        reason: &str,
    ) {
        if reconciliation.results.is_empty() {
            return;
        }
        self.notify(|| AgentProgressEvent::ToolPolicyChanged {
            reconciliation: reconciliation.clone(),
            reason: reason.to_string(),
        });
    }

    pub(super) fn mark_turn_in_flight(&self) {
        self.turn_in_flight.store(true, Ordering::SeqCst);
    }

    pub(super) fn clear_turn_in_flight(&self) {
        self.turn_in_flight.store(false, Ordering::SeqCst);
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
        let reconciliation = self
            .tool_registry
            .apply_tool_policy_mutations(mutations, mode);
        self.notify_tool_policy_changed(&reconciliation, "immediate");
        Some(reconciliation)
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
                self.clear_turn_in_flight();
                return None;
            }
            std::mem::take(&mut *pending)
        };
        self.clear_turn_in_flight();
        let reconciliation = self
            .tool_registry
            .apply_tool_policy_mutations(&mutations, ToolPolicyApplyMode::AtNextTurnBoundary);
        self.notify_tool_policy_changed(&reconciliation, "turn_boundary");
        Some(reconciliation)
    }
}
