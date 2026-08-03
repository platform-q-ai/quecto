use super::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::tool::{
    ToolDefinition, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus,
    ToolPolicyReconciliation,
};
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolCatalogueEntry};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

#[derive(Debug, Default, Clone)]
pub(super) struct ToolPolicyState {
    pub(super) disabled_tools: HashSet<String>,
    pub(super) enabled_tools: HashSet<String>,
    pub(super) scopes: HashMap<String, ProfileAvailabilityScope>,
}

impl ToolPolicyState {
    pub(super) fn is_model_visible(
        &self,
        name: &str,
        profile: crate::domain::tool::ToolProfileContext,
        catalogue_entry: Option<&ToolCatalogueEntry>,
    ) -> bool {
        if let Some(entry) = catalogue_entry
            && entry.explicit_restriction.is_some()
        {
            return false;
        }
        if let Some(scope) = self.scopes.get(name) {
            return Self::scope_allows(*scope, profile);
        }
        if let Some(entry) = catalogue_entry {
            let profile_enabled = match profile {
                crate::domain::tool::ToolProfileContext::Parent => entry.effective_parent_enabled,
                crate::domain::tool::ToolProfileContext::Child => entry.effective_child_enabled,
            };
            return (profile_enabled || self.enabled_tools.contains(name))
                && entry.explicit_restriction.is_none()
                && (entry.profile_scope.is_some() || !self.disabled_tools.contains(name));
        }
        !self.disabled_tools.contains(name) || self.enabled_tools.contains(name)
    }

    pub(super) fn blocks_execution(
        &self,
        name: &str,
        profile: crate::domain::tool::ToolProfileContext,
    ) -> bool {
        self.scopes.get(name).map_or_else(
            || self.disabled_tools.contains(name),
            |scope| !Self::scope_allows(*scope, profile),
        )
    }

    pub(super) fn record_applied(&mut self, name: &str, scope: ProfileAvailabilityScope) {
        self.scopes.insert(name.to_string(), scope);
        match scope {
            ProfileAvailabilityScope::Both => {
                self.disabled_tools.remove(name);
                self.enabled_tools.insert(name.to_string());
            }
            ProfileAvailabilityScope::Parent => {
                self.disabled_tools.remove(name);
                self.enabled_tools.remove(name);
            }
            ProfileAvailabilityScope::Child | ProfileAvailabilityScope::None => {
                self.disabled_tools.insert(name.to_string());
                self.enabled_tools.remove(name);
            }
        }
    }

    fn scope_allows(
        scope: ProfileAvailabilityScope,
        profile: crate::domain::tool::ToolProfileContext,
    ) -> bool {
        match profile {
            crate::domain::tool::ToolProfileContext::Parent => scope.allows_parent(),
            crate::domain::tool::ToolProfileContext::Child => scope.allows_child(),
        }
    }
}

impl AgentLoopImpl {
    pub(super) fn current_tool_definitions(&self) -> Vec<ToolDefinition> {
        let catalogue_entries = self.tool_catalogue_entries();
        let policy = self
            .tool_policy_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if catalogue_entries.is_empty() {
            return self
                .tool_catalog()
                .definitions_for(self.tool_profile_context)
                .iter()
                .filter(|definition| {
                    policy.is_model_visible(
                        definition.name.as_ref(),
                        self.tool_profile_context,
                        None,
                    )
                })
                .cloned()
                .collect();
        }
        catalogue_entries
            .into_iter()
            .filter(|entry| {
                policy.is_model_visible(entry.name.as_ref(), self.tool_profile_context, Some(entry))
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
        let mut policy = self
            .tool_policy_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for result in &reconciliation.results {
            if result.status == ToolPolicyMutationStatus::Applied {
                policy.record_applied(result.name.as_ref(), result.requested_scope);
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
