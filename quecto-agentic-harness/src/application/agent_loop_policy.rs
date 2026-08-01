use super::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::tool::{
    ToolDefinition, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationResult,
    ToolPolicyMutationStatus, ToolPolicyReconciliation,
};
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolCatalogueEntry};
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
        let catalogue_entries = self.tool_catalogue_entries();
        if catalogue_entries.is_empty() {
            return self
                .tool_catalog()
                .definitions_for(self.tool_profile_context)
                .iter()
                .filter(|definition| {
                    !disabled.contains(definition.name.as_ref())
                        || enabled.contains(definition.name.as_ref())
                })
                .cloned()
                .collect();
        }
        catalogue_entries
            .into_iter()
            .filter(|entry| {
                let profile_enabled = match self.tool_profile_context {
                    crate::domain::tool::ToolProfileContext::Parent => {
                        entry.effective_parent_enabled
                    }
                    crate::domain::tool::ToolProfileContext::Child => entry.effective_child_enabled,
                };
                (profile_enabled || enabled.contains(entry.name.as_ref()))
                    && !disabled.contains(entry.name.as_ref())
                    && entry.explicit_restriction.is_none()
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
        for result in &reconciliation.results {
            if result.status != ToolPolicyMutationStatus::Applied {
                continue;
            }
            if result.requested_scope.allows_parent() {
                disabled.remove(result.name.as_str());
                enabled.insert(result.name.to_string());
            } else {
                disabled.insert(result.name.to_string());
                enabled.remove(result.name.as_str());
            }
        }
    }

    fn entry_scope_ceiling(entry: &ToolCatalogueEntry) -> ProfileAvailabilityScope {
        let default = ProfileAvailabilityScope::from_enabled(entry.default_enabled);
        let configured = entry
            .configured_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let session = entry
            .session_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let restriction = if entry.explicit_restriction.is_some() {
            ProfileAvailabilityScope::None
        } else {
            ProfileAvailabilityScope::Both
        };
        default
            .intersection(configured)
            .intersection(session)
            .intersection(restriction)
    }

    pub fn queue_tool_policy_mutation(&self, mutations: &[ToolPolicyMutation]) {
        let mut pending = self
            .pending_tool_policy_mutations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.extend_from_slice(mutations);
    }

    fn drain_pending_tool_policy_mutations(
        &self,
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

        let mut before = self.tool_catalogue_entries();
        if before.is_empty() {
            before = self
                .tool_catalog()
                .definitions()
                .iter()
                .map(|definition| ToolCatalogueEntry {
                    stable_id: definition.name.to_string().into(),
                    name: definition.name.to_string().into(),
                    label: definition.name.to_string().into(),
                    description: definition.description.to_string().into(),
                    input_schema: definition.parameters_schema.to_string().into(),
                    source: crate::domain::tool_descriptor::ToolSource::Runtime,
                    owner: "runtime".into(),
                    provider_id: "runtime".into(),
                    version: None,
                    lifecycle: crate::domain::tool_descriptor::ToolLifecycleKind::RuntimeLoadable,
                    configurable: true,
                    default_enabled: true,
                    configured_enabled: None,
                    profile_enabled: None,
                    profile_scope: None,
                    session_enabled: None,
                    explicit_restriction: None,
                    runtime_availability: crate::domain::tool_descriptor::ToolAvailability::Enabled,
                    effective_enabled: true,
                    effective_scope: crate::domain::tool_descriptor::ProfileAvailabilityScope::Both,
                    effective_parent_enabled: true,
                    effective_child_enabled: true,
                    health: crate::domain::tool_descriptor::ToolHealth::Ok,
                })
                .collect();
        }
        {
            let disabled = self
                .runtime_disabled_tools
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for mutation in &mutations {
                if mutation.availability.is_enabled()
                    && before.iter().all(|entry| entry.name != mutation.name)
                    && disabled.contains(mutation.name.as_str())
                {
                    before.push(ToolCatalogueEntry {
                        stable_id: mutation.name.to_string().into(),
                        name: mutation.name.to_string().into(),
                        label: mutation.name.to_string().into(),
                        description: "".into(),
                        input_schema: r#"{"type":"object"}"#.into(),
                        source: crate::domain::tool_descriptor::ToolSource::Runtime,
                        owner: "runtime".into(),
                        provider_id: "runtime".into(),
                        version: None,
                        lifecycle:
                            crate::domain::tool_descriptor::ToolLifecycleKind::RuntimeLoadable,
                        configurable: true,
                        default_enabled: true,
                        configured_enabled: None,
                        profile_enabled: None,
                        profile_scope: None,
                        session_enabled: None,
                        explicit_restriction: None,
                        runtime_availability:
                            crate::domain::tool_descriptor::ToolAvailability::Disabled,
                        effective_enabled: false,
                        effective_scope:
                            crate::domain::tool_descriptor::ProfileAvailabilityScope::None,
                        effective_parent_enabled: false,
                        effective_child_enabled: false,
                        health: crate::domain::tool_descriptor::ToolHealth::Disabled,
                    });
                }
            }
        }
        let disabled_before = self
            .runtime_disabled_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        {
            let mut disabled = self
                .runtime_disabled_tools
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut enabled = self
                .runtime_enabled_tools
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for mutation in &mutations {
                let before_entry = before.iter().find(|entry| entry.name == mutation.name);
                if before_entry.is_some_and(|entry| entry.explicit_restriction.is_none()) {
                    if mutation.scope.allows_parent() {
                        disabled.remove(mutation.name.as_str());
                        enabled.insert(mutation.name.to_string());
                    } else {
                        disabled.insert(mutation.name.to_string());
                        enabled.remove(mutation.name.as_str());
                    }
                } else if mutation.scope == ProfileAvailabilityScope::None {
                    disabled.insert(mutation.name.to_string());
                    enabled.remove(mutation.name.as_str());
                }
            }
        }
        let mut after = self.tool_catalogue_entries();
        if after.is_empty() {
            after = before.clone();
        }
        let results = mutations
            .iter()
            .map(|mutation| {
                let before_entry = before
                    .iter()
                    .find(|entry| entry.name == mutation.name)
                    .cloned();
                let mut after_entry = after
                    .iter()
                    .find(|entry| entry.name == mutation.name)
                    .cloned();
                let status = match before_entry.as_ref() {
                    None => ToolPolicyMutationStatus::UnknownTool,
                    Some(entry)
                        if !mutation
                            .scope
                            .is_subset_of(Self::entry_scope_ceiling(entry)) =>
                    {
                        ToolPolicyMutationStatus::BlockedByRestriction
                    }
                    Some(entry)
                        if entry.effective_scope == mutation.scope
                            && !(mutation.scope.allows_parent()
                                && disabled_before.contains(mutation.name.as_str())) =>
                    {
                        ToolPolicyMutationStatus::AlreadyInState
                    }
                    Some(_) => ToolPolicyMutationStatus::Applied,
                };
                if let Some(entry) = after_entry.as_mut() {
                    match status {
                        ToolPolicyMutationStatus::Applied => {
                            entry.profile_scope = Some(mutation.scope);
                            entry.profile_enabled = Some(mutation.scope.is_enabled());
                            entry.effective_scope = mutation.scope;
                            entry.effective_parent_enabled = mutation.scope.allows_parent();
                            entry.effective_child_enabled = mutation.scope.allows_child();
                            entry.effective_enabled = mutation.scope.is_enabled();
                            entry.runtime_availability = mutation.availability;
                        }
                        ToolPolicyMutationStatus::BlockedByRestriction => {
                            entry.effective_scope = ProfileAvailabilityScope::None;
                            entry.effective_parent_enabled = false;
                            entry.effective_child_enabled = false;
                            entry.effective_enabled = false;
                        }
                        _ => {}
                    }
                }
                ToolPolicyMutationResult {
                    name: mutation.name.clone(),
                    requested_availability: mutation.availability,
                    requested_scope: mutation.scope,
                    status,
                    before: before_entry,
                    after: after_entry,
                    reason: mutation.reason.clone(),
                }
            })
            .collect();
        let reconciliation = ToolPolicyReconciliation {
            mode: ToolPolicyApplyMode::AtNextTurnBoundary,
            results,
        };
        self.notify_tool_policy_changed(&reconciliation, "turn_boundary");
        Some(reconciliation)
    }

    pub(super) fn drain_tool_policy_mutations_at_internal_boundary(
        &self,
    ) -> Option<ToolPolicyReconciliation> {
        self.drain_pending_tool_policy_mutations(false)
    }

    pub fn drain_tool_policy_mutations_at_boundary(&mut self) -> Option<ToolPolicyReconciliation> {
        self.drain_pending_tool_policy_mutations(true)
    }
}
