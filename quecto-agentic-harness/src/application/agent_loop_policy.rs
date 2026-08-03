use super::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::tool::{
    ChildToolPolicyPropagation, ToolDefinition, ToolPolicyApplyMode, ToolPolicyMutation,
    ToolPolicyMutationStatus, ToolPolicyReconciliation,
};
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolCatalogueEntry};
use std::collections::{BTreeMap, HashMap, HashSet};
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

    pub(super) fn apply_to_catalogue_entry(&self, entry: &mut ToolCatalogueEntry) {
        let name = entry.name.as_ref();
        let scope = self.scopes.get(name).copied().or_else(|| {
            if self.disabled_tools.contains(name) {
                Some(ProfileAvailabilityScope::None)
            } else if self.enabled_tools.contains(name) {
                Some(ProfileAvailabilityScope::Both)
            } else {
                None
            }
        });
        if let Some(scope) = scope {
            entry.session_enabled = Some(scope.is_enabled());
            entry.effective_scope = scope;
            entry.effective_parent_enabled = scope.allows_parent();
            entry.effective_child_enabled = scope.allows_child();
            entry.effective_enabled = scope.is_enabled();
            entry.runtime_availability = if scope.is_enabled() {
                crate::domain::tool_descriptor::ToolAvailability::Enabled
            } else {
                crate::domain::tool_descriptor::ToolAvailability::Disabled
            };
            entry.health = if scope.is_enabled() {
                crate::domain::tool_descriptor::ToolHealth::Ok
            } else {
                crate::domain::tool_descriptor::ToolHealth::Disabled
            };
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
    pub fn tool_catalogue_entries(
        &self,
    ) -> Vec<crate::domain::tool_descriptor::ToolCatalogueEntry> {
        let mut entries = self.tool_catalog().catalogue_entries();
        let policy = self
            .tool_policy_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for entry in &mut entries {
            policy.apply_to_catalogue_entry(entry);
        }
        entries
    }

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

    pub(super) fn refresh_spawn_inherited_child_policy_snapshot(&self) {
        let snapshot: BTreeMap<_, _> = self
            .tool_catalogue_entries()
            .into_iter()
            .map(|tool| (tool.name.into_owned(), tool.effective_scope))
            .collect();
        self.extension_tool_registry()
            .set_inherited_child_policy_snapshot_for_spawn(snapshot);
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
        self.request_tool_policy_mutation_with_child_propagation(mutations, mode, true)
    }

    pub fn request_tool_policy_mutation_with_child_propagation(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
        propagate_to_children: bool,
    ) -> Option<ToolPolicyReconciliation> {
        let in_flight = self.turn_in_flight.load(Ordering::SeqCst);
        let should_queue = (mode == ToolPolicyApplyMode::AtNextTurnBoundary && in_flight)
            || (mode == ToolPolicyApplyMode::ImmediateIfIdle && in_flight);
        if should_queue {
            self.queue_tool_policy_mutation(mutations);
            if propagate_to_children && let Some(propagator) = &self.tool_policy_child_propagator {
                let child_propagation = propagator.propagate_tool_policy_to_children(
                    mutations,
                    ToolPolicyApplyMode::AtNextTurnBoundary,
                );
                let child_propagation_count = child_propagation.len();
                self.pending_tool_policy_child_propagation
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend(child_propagation);
                *self
                    .pending_tool_policy_last_child_propagation_count
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = child_propagation_count;
            }
            return None;
        }
        let mut reconciliation = self
            .tool_registry
            .apply_tool_policy_mutations(mutations, mode);
        if propagate_to_children && let Some(propagator) = &self.tool_policy_child_propagator {
            reconciliation.child_propagation =
                propagator.propagate_tool_policy_to_children(mutations, mode);
        }
        self.record_applied_tool_policy_overlay(&reconciliation);
        self.refresh_spawn_inherited_child_policy_snapshot();
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

    pub fn recent_tool_policy_child_propagation_for_response(
        &self,
    ) -> Option<Vec<ChildToolPolicyPropagation>> {
        let child_propagation = self
            .pending_tool_policy_child_propagation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = *self
            .pending_tool_policy_last_child_propagation_count
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if child_propagation.is_empty() || count == 0 {
            return None;
        }
        let start = child_propagation.len().saturating_sub(count);
        Some(child_propagation[start..].to_vec())
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

        let mut reconciliation = self
            .tool_registry
            .apply_tool_policy_mutations(&mutations, ToolPolicyApplyMode::AtNextTurnBoundary);
        let prompt_child_propagation = std::mem::take(
            &mut *self
                .pending_tool_policy_child_propagation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        if prompt_child_propagation.is_empty() {
            if let Some(propagator) = &self.tool_policy_child_propagator {
                reconciliation.child_propagation = propagator.propagate_tool_policy_to_children(
                    &mutations,
                    ToolPolicyApplyMode::AtNextTurnBoundary,
                );
            }
        } else {
            reconciliation.child_propagation = prompt_child_propagation;
        }
        self.record_applied_tool_policy_overlay(&reconciliation);
        self.refresh_spawn_inherited_child_policy_snapshot();
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
