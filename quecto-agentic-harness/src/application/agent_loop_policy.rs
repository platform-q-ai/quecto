use super::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::tool::{
    ToolDefinition, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus,
    ToolPolicyReconciliation,
};
use crate::domain::tool_descriptor::ToolCatalogueEntry;
use std::sync::atomic::Ordering;

impl AgentLoopImpl {
    pub(super) fn current_tool_definitions(&self) -> Vec<ToolDefinition> {
        let disabled = self
            .runtime_disabled_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let enabled = self
            .runtime_enabled_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let scopes = self
            .runtime_policy_scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let catalogue_entries = self.tool_catalogue_entries();
        if catalogue_entries.is_empty() {
            return self
                .tool_catalog()
                .definitions_for(self.tool_profile_context)
                .iter()
                .filter(|definition| {
                    scopes.get(definition.name.as_ref()).map_or(
                        !disabled.contains(definition.name.as_ref()),
                        |scope| match self.tool_profile_context {
                            crate::domain::tool::ToolProfileContext::Parent => {
                                scope.allows_parent()
                            }
                            crate::domain::tool::ToolProfileContext::Child => scope.allows_child(),
                        },
                    ) || enabled.contains(definition.name.as_ref())
                })
                .cloned()
                .collect();
        }
        catalogue_entries
            .into_iter()
            .filter(|entry| {
                let profile_enabled = scopes.get(entry.name.as_ref()).map_or_else(
                    || match self.tool_profile_context {
                        crate::domain::tool::ToolProfileContext::Parent => {
                            entry.effective_parent_enabled
                        }
                        crate::domain::tool::ToolProfileContext::Child => {
                            entry.effective_child_enabled
                        }
                    },
                    |scope| match self.tool_profile_context {
                        crate::domain::tool::ToolProfileContext::Parent => scope.allows_parent(),
                        crate::domain::tool::ToolProfileContext::Child => scope.allows_child(),
                    },
                );
                (profile_enabled || enabled.contains(entry.name.as_ref()))
                    && entry.explicit_restriction.is_none()
                    && (entry.profile_scope.is_some() || !disabled.contains(entry.name.as_ref()))
            })
            .map(|entry| ToolDefinition {
                name: entry.name,
                description: entry.description,
                parameters_schema: entry.input_schema,
            })
            .collect()
    }

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
        if mode == ToolPolicyApplyMode::AtNextTurnBoundary
            || (mode == ToolPolicyApplyMode::ImmediateIfIdle
                && self.turn_in_flight.load(Ordering::SeqCst))
        {
            self.queue_tool_policy_mutation(mutations);
            return None;
        }
        let reconciliation = self
            .tool_registry
            .apply_tool_policy_mutations(mutations, mode);
        self.record_applied_tool_policy_overlay(&reconciliation);
        self.notify_tool_policy_changed(&reconciliation, "immediate");
        Some(reconciliation)
    }

    fn record_applied_tool_policy_overlay(&self, reconciliation: &ToolPolicyReconciliation) {
        let mut disabled = self
            .runtime_disabled_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut enabled = self
            .runtime_enabled_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut scopes = self
            .runtime_policy_scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for result in &reconciliation.results {
            if result.status != ToolPolicyMutationStatus::Applied {
                continue;
            }
            scopes.insert(result.name.to_string(), result.requested_scope);
            if result.requested_scope.allows_parent() {
                disabled.remove(result.name.as_str());
                enabled.insert(result.name.to_string());
            } else {
                disabled.insert(result.name.to_string());
                enabled.remove(result.name.as_str());
            }
        }
    }

    pub fn queue_tool_policy_mutation(&self, mutations: &[ToolPolicyMutation]) {
        let mut pending = self
            .pending_tool_policy_mutations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.extend_from_slice(mutations);
    }

    fn drain_pending_tool_policy_mutations(
        &mut self,
        clear_in_flight: bool,
    ) -> Option<ToolPolicyReconciliation> {
        let mutations = {
            let mut pending = self
                .pending_tool_policy_mutations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pending.is_empty() {
                if clear_in_flight {
                    self.clear_turn_in_flight();
                }
                return None;
            }
            std::mem::take(&mut *pending)
        };
        if clear_in_flight {
            self.clear_turn_in_flight();
        }

        // Apply through the same registry path as immediate mutations so
        // catalogue entries, runtime availability, and event `after` snapshots
        // stay consistent. Overlays are updated only for Applied results.
        let reconciliation = self
            .tool_registry
            .apply_tool_policy_mutations(&mutations, ToolPolicyApplyMode::AtNextTurnBoundary);
        self.record_applied_tool_policy_overlay(&reconciliation);
        self.notify_tool_policy_changed(&reconciliation, "turn_boundary");
        Some(reconciliation)
    }

    pub(super) fn drain_tool_policy_mutations_at_internal_boundary(
        &mut self,
    ) -> Option<ToolPolicyReconciliation> {
        self.drain_pending_tool_policy_mutations(false)
    }

    pub fn drain_tool_policy_mutations_at_boundary(&mut self) -> Option<ToolPolicyReconciliation> {
        self.drain_pending_tool_policy_mutations(true)
    }
}
