use std::sync::Arc;

use super::super::registration::ToolRegistration;
use super::ToolRegistryImpl;
use crate::application::tools::ports::ToolGuard;
use crate::domain::tool_policy::value_objects::tool_descriptor::{
    ProfileAvailabilityScope, ToolCatalogueEntry,
};

impl ToolRegistryImpl {
    pub(super) fn restriction_ceiling(
        &self,
        entry: &ToolCatalogueEntry,
    ) -> ProfileAvailabilityScope {
        let default = ProfileAvailabilityScope::from_enabled(entry.default_enabled);
        // Configured/persisted policy is a user preference, not a hard ceiling
        // for future policy mutations. It participates in effective_scope(),
        // but requests must be able to replace a durable narrow preference
        // without manual config edits.
        let session = entry
            .session_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let inherited = self
            .metadata
            .get(entry.name.as_ref())
            .and_then(|m| m.inherited_scope)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let restriction = if entry.explicit_restriction.is_some() {
            ProfileAvailabilityScope::None
        } else {
            ProfileAvailabilityScope::Both
        };
        default
            .intersection(session)
            .intersection(inherited)
            .intersection(restriction)
    }
}

/// What follows this registry's policy (S4c): told, each time the policy
/// or the guards change, the registry as it now stands, to read what it
/// needs through [`ToolRegistryImpl::parent_may_call`] and
/// [`ToolRegistryImpl::guards`]. The registry knows nothing of what an
/// observer is for.
pub trait ToolPolicyObserver: Send + Sync {
    fn policy_changed(&self, registry: &ToolRegistryImpl);
}

impl ToolRegistryImpl {
    /// `observer` follows this registry's policy and guards from now on,
    /// told of them as they stand now first.
    pub fn observe_policy(&mut self, observer: Arc<dyn ToolPolicyObserver>) {
        observer.policy_changed(self);
        self.policy_observers.push(observer);
    }

    /// Tells every observer the registry as it now stands.
    pub(super) fn notify_policy_observers(&self) {
        for observer in &self.policy_observers {
            observer.policy_changed(self);
        }
    }

    /// Whether a parent may call `name` now: it is registered, and the
    /// policy's effective scope allows a parent.
    pub fn parent_may_call(&self, name: &str) -> bool {
        self.tools.contains_key(name) && {
            let metadata = self
                .metadata
                .get(name)
                .cloned()
                .unwrap_or_else(ToolRegistration::official_native);
            Self::effective_scope(&metadata).allows_parent()
        }
    }

    /// The guards every call passes, in their order.
    pub fn guards(&self) -> &[Arc<dyn ToolGuard>] {
        &self.guards
    }
}
