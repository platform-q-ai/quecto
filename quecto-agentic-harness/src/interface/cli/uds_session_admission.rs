use super::AgentSession;

impl AgentSession {
    /// Replace the advisory snapshot atomically with the composed runtime generation.
    pub fn set_admission_warnings(&mut self, slots: &[String]) {
        let mut unique = std::collections::BTreeSet::new();
        self.admission_warnings = slots
            .iter()
            .filter(|slot| unique.insert((*slot).clone()))
            .map(|slot| crate::domain::state_snapshot::AdmissionBindingWarning::new(slot))
            .collect();
        self.bump_visible_generation();
    }

    pub fn observe_runtime(&mut self, store: crate::application::ports::RuntimeSnapshotStore) {
        self.runtime_store = Some(store);
        self.refresh_admission_warnings();
    }

    pub(crate) fn current_admission_warnings(
        &self,
    ) -> Vec<crate::domain::state_snapshot::AdmissionBindingWarning> {
        let Some(store) = &self.runtime_store else {
            return self.admission_warnings.clone();
        };
        let Some(runtime) = store.current() else {
            return self.admission_warnings.clone();
        };
        let mut seen = std::collections::BTreeSet::new();
        runtime
            .admission_binding_diagnostic
            .unbound_slots
            .iter()
            .filter(|slot| seen.insert((*slot).clone()))
            .map(|slot| crate::domain::state_snapshot::AdmissionBindingWarning::new(slot))
            .collect()
    }

    pub fn refresh_admission_warnings(&mut self) {
        let current = self.current_admission_warnings();
        if current != self.admission_warnings {
            self.admission_warnings = current;
            self.bump_visible_generation();
        }
    }
}
