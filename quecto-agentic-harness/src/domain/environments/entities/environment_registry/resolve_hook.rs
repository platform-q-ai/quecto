//! Test-only: run a closure once, right after the next `resolve` returns
//! its answer and before the caller acts on it, so a test can change the
//! registry in the window between a lookup and a claim (a ref reused by
//! another session).

use std::sync::{Arc, Mutex};

use super::EnvironmentRegistry;

type Hook = Box<dyn FnOnce(&EnvironmentRegistry) + Send>;

#[derive(Clone, Default)]
pub(super) struct ResolveHook(Arc<Mutex<Option<Hook>>>);

impl std::fmt::Debug for ResolveHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResolveHook")
    }
}

impl ResolveHook {
    pub(super) fn fire(&self, registry: &EnvironmentRegistry) {
        let hook = self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(hook) = hook {
            hook(registry);
        }
    }
}

impl EnvironmentRegistry {
    /// Run `hook` once, after the next `resolve` and before its caller
    /// acts on the answer.
    pub(crate) fn after_next_resolve(
        &self,
        hook: impl FnOnce(&EnvironmentRegistry) + Send + 'static,
    ) {
        *self
            .after_resolve
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Box::new(hook));
    }
}
