//! Whether this session may spawn (S4c), as its tool registry's policy
//! says now: the registry sets it each time its policy changes, and the
//! board's coordinator launcher reads it before each launch, so `start`
//! never launches what the session's own `spawn` could not.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::registration::ToolRegistration;
use super::super::registry::ToolRegistryImpl;

/// The session's spawn permission, shared: closed until a registry
/// watching it says otherwise.
#[derive(Debug, Clone, Default)]
pub struct SpawnPermit(Arc<AtomicBool>);

impl SpawnPermit {
    /// Whether the session's policy allows spawn now.
    pub fn allowed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn set(&self, allowed: bool) {
        self.0.store(allowed, Ordering::SeqCst);
    }
}

impl ToolRegistryImpl {
    /// `permit` follows this registry's policy from now on: whether a
    /// parent may call `spawn`, by the scope its own calls are judged by.
    pub fn watch_spawn_permit(&mut self, permit: SpawnPermit) {
        self.spawn_permit = Some(permit);
        self.refresh_spawn_permit();
    }

    /// Sets the watched permit, if any, to this registry's policy now.
    pub(in crate::infrastructure::tools) fn refresh_spawn_permit(&self) {
        let Some(permit) = &self.spawn_permit else {
            return;
        };
        let allowed = self.tools.contains_key("spawn") && {
            let metadata = self
                .metadata
                .get("spawn")
                .cloned()
                .unwrap_or_else(ToolRegistration::official_native);
            Self::effective_scope(&metadata).allows_parent()
        };
        permit.set(allowed);
    }
}

#[cfg(test)]
#[path = "spawn_permit_tests.rs"]
mod tests;
