//! The agent loop's context-gauge wrappers and their test hooks (split
//! from `agent_loop.rs` for the 750-line cap, #2212). The gauge itself and
//! its calibration live in `application::context`.

use super::AgentLoopImpl;

impl AgentLoopImpl {
    pub(super) fn reconcile_context_gauge(&self, estimate: usize) -> usize {
        self.context_manager.reconcile_context_gauge(estimate)
    }

    pub(super) fn observe_provider_context_gauge(
        &self,
        reported_tokens: usize,
        estimate_at_call: usize,
    ) {
        self.context_manager
            .observe_provider_context_gauge(reported_tokens, estimate_at_call);
    }

    pub(super) fn observe_estimated_context_gauge(&self, estimate: usize) {
        self.context_manager
            .observe_estimated_context_gauge(estimate);
    }

    #[doc(hidden)]
    pub fn reconcile_context_gauge_for_test(&self, estimate: usize) -> usize {
        self.reconcile_context_gauge(estimate)
    }

    #[doc(hidden)]
    pub fn observe_provider_context_gauge_for_test(
        &self,
        reported_tokens: usize,
        estimate_at_call: usize,
    ) {
        self.observe_provider_context_gauge(reported_tokens, estimate_at_call);
    }

    /// Poison the context-gauge mutex so coverage exercises the
    /// `unwrap_or_else(|e| e.into_inner())` recovery paths (#1128).
    #[cfg(test)]
    pub(super) fn poison_context_gauge_lock_for_test(&self) {
        self.context_manager.poison_context_gauge_lock_for_test();
    }

    /// Drive all three gauge entry points against a poisoned mutex.
    #[cfg(test)]
    pub(super) fn exercise_poisoned_context_gauge_for_test(&self) {
        self.poison_context_gauge_lock_for_test();
        assert_eq!(self.reconcile_context_gauge(42), 42);
        self.observe_provider_context_gauge(1_000, 100);
        assert_eq!(self.reconcile_context_gauge(80), 980);
        self.observe_estimated_context_gauge(1);
        assert_eq!(
            self.reconcile_context_gauge(80),
            980,
            "estimate-only must not clobber provider truth after poison recovery"
        );
    }
}
