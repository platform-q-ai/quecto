use super::agent_loop::AgentLoopImpl;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::tool::{
    ToolDefinition, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus,
    ToolPolicyReconciliation, ToolPolicyRequest,
};
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolCatalogueEntry};
use crate::domain::tool_id::stable_tool_id;
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
        let mut snapshot = BTreeMap::new();
        for tool in self.tool_catalogue_entries() {
            let name = tool.name.into_owned();
            let stable_id = tool.stable_id.into_owned();
            let scope = tool.effective_scope;
            let is_generated_legacy_stable_id =
                stable_id == stable_tool_id(tool.source, tool.provider_id.as_ref(), &name);
            snapshot.insert(stable_id, scope);
            if is_generated_legacy_stable_id {
                snapshot.insert(name, scope);
            }
        }
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

    pub fn set_tool_policy_persistence(
        &mut self,
        persistence: Option<super::agent_loop::ToolPolicyPersistence>,
    ) {
        self.tool_policy_persistence = persistence;
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
            self.queue_tool_policy_request(ToolPolicyRequest::patch(mutations.to_vec()));
            return None;
        }
        let reconciliation =
            self.request_tool_policy(ToolPolicyRequest::patch(mutations.to_vec()), mode)?;
        Some(reconciliation)
    }

    pub fn request_tool_policy(
        &mut self,
        request: ToolPolicyRequest,
        mode: ToolPolicyApplyMode,
    ) -> Option<ToolPolicyReconciliation> {
        if mode == ToolPolicyApplyMode::AtNextTurnBoundary
            || (mode == ToolPolicyApplyMode::ImmediateIfIdle
                && self.turn_in_flight.load(Ordering::SeqCst))
        {
            self.queue_tool_policy_request(request);
            return None;
        }
        let mut reconciliation = self.tool_registry.apply_tool_policy_request(&request, mode);
        let persist_error = if request.persist {
            self.tool_policy_persistence
                .as_ref()
                .and_then(|persist| persist(&reconciliation).err())
        } else {
            None
        };
        if let Some(error) = persist_error {
            self.mark_tool_policy_persistence_failed(&mut reconciliation, &error);
        } else {
            self.record_applied_tool_policy_overlay(&reconciliation);
            if request.persist && self.tool_policy_persistence.is_some() {
                self.tool_registry
                    .record_persisted_tool_policy_results(&reconciliation);
            }
            self.refresh_spawn_inherited_child_policy_snapshot();
            self.notify_tool_policy_changed(&reconciliation, "immediate");
        }
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

    fn mark_tool_policy_persistence_failed(
        &self,
        reconciliation: &mut ToolPolicyReconciliation,
        error: &str,
    ) {
        for result in &mut reconciliation.results {
            if matches!(
                result.status,
                ToolPolicyMutationStatus::Applied | ToolPolicyMutationStatus::AlreadyInState
            ) {
                result.status = ToolPolicyMutationStatus::PersistenceFailed;
                result.reason = format!(
                    "{}; persisted tool policy update failed: {}",
                    result.reason, error
                );
            }
        }
    }

    pub fn queue_tool_policy_mutation(&self, mutations: &[ToolPolicyMutation]) {
        self.queue_tool_policy_request(ToolPolicyRequest::patch(mutations.to_vec()));
    }

    pub fn queue_tool_policy_request(&self, request: ToolPolicyRequest) {
        let mut pending = self
            .pending_tool_policy_requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.push(request);
    }

    fn drain_pending_tool_policy_requests(
        &mut self,
        clear_in_flight: bool,
    ) -> Option<ToolPolicyReconciliation> {
        let requests = {
            let mut pending = self
                .pending_tool_policy_requests
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

        let mut combined = ToolPolicyReconciliation {
            mode: ToolPolicyApplyMode::AtNextTurnBoundary,
            results: Vec::new(),
            correlation_id: None,
        };
        for request in requests {
            if combined.correlation_id.is_none() {
                combined.correlation_id = request.correlation_id.clone();
            }
            let mut reconciliation = self
                .tool_registry
                .apply_tool_policy_request(&request, ToolPolicyApplyMode::AtNextTurnBoundary);
            self.record_applied_tool_policy_overlay(&reconciliation);
            let persist_error = if request.persist {
                self.tool_policy_persistence
                    .as_ref()
                    .and_then(|persist| persist(&reconciliation).err())
            } else {
                None
            };
            if let Some(error) = persist_error {
                self.mark_tool_policy_persistence_failed(&mut reconciliation, &error);
            } else {
                if request.persist && self.tool_policy_persistence.is_some() {
                    self.tool_registry
                        .record_persisted_tool_policy_results(&reconciliation);
                }
                self.notify_tool_policy_changed(&reconciliation, "turn_boundary");
            }
            combined.results.extend(reconciliation.results);
        }
        self.refresh_spawn_inherited_child_policy_snapshot();
        Some(combined)
    }

    pub(super) fn drain_tool_policy_mutations_at_internal_boundary(
        &mut self,
    ) -> Option<ToolPolicyReconciliation> {
        self.drain_pending_tool_policy_requests(false)
    }

    pub fn drain_tool_policy_mutations_at_boundary(&mut self) -> Option<ToolPolicyReconciliation> {
        self.drain_pending_tool_policy_requests(true)
    }
}
