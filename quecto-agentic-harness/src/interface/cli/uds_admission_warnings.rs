//! Startup presentation of the published runtime's advisory admission slots.

pub(super) fn publish_startup_warnings(
    store: &crate::application::ports::RuntimeSnapshotStore,
) -> Vec<String> {
    let slots = store
        .current()
        .map(|snapshot| snapshot.admission_binding_diagnostic.unbound_slots.clone())
        .unwrap_or_default();
    let unique: Vec<String> = slots
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for slot in &unique {
        eprintln!(
            "warning: {}",
            crate::domain::state_snapshot::AdmissionBindingWarning::new(slot).message
        );
    }
    unique
}

/// Overlay the latest published generation for a busy/connect-time inspector read.
pub(super) fn overlay_current(
    state: &mut super::protocol::SessionState,
    store: &crate::application::ports::RuntimeSnapshotStore,
) {
    if let Some(runtime) = store.current() {
        state.admission_warnings = runtime
            .admission_binding_diagnostic
            .unbound_slots
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|slot| crate::domain::state_snapshot::AdmissionBindingWarning::new(slot))
            .collect();
    }
}
