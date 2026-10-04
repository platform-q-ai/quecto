use super::AgentSession;

impl AgentSession {
    /// The warnings the startup model drew (#2435, #2126): `get_state`
    /// carries them, so a client that never saw the agent's stderr (the
    /// TUI) can show them.
    /// They are about the session's model when set (the startup model): a
    /// later switch to another model clears them.
    pub fn set_startup_warnings(&mut self, warnings: &[String]) {
        let about: Vec<(String, String)> = warnings
            .iter()
            .map(|warning| (self.model.clone(), warning.clone()))
            .collect();
        if self.startup_warnings != about {
            self.startup_warnings = about;
            self.bump_visible_generation();
        }
    }

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

    /// Watch this agent's configured extensions (#2446).
    pub fn observe_extensions(
        &mut self,
        extensions: crate::interface::cli::uds_extensions::Extensions,
    ) {
        self.extensions = extensions;
        self.bump_visible_generation();
    }

    /// What this agent's configured extensions warn of (#2446).
    pub(crate) fn extension_warnings(&self) -> Vec<String> {
        self.extensions
            .iter()
            .flat_map(|extensions| extensions.warnings())
            .collect()
    }

    /// The generation clients compare: the tracker's own, advanced by every
    /// change of an extension's state, so a `since` read never hides one.
    pub(crate) fn visible_generation(&self) -> u64 {
        let extensions = self
            .extensions
            .as_ref()
            .map_or(0, |extensions| extensions.revision());
        self.generation.wrapping_add(extensions).max(1)
    }
}
