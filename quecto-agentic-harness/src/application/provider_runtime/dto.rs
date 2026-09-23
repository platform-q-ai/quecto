use crate::application::providers::ports::LlmProvider;
use std::sync::Arc;

/// Sanitized admission advisory for usable router slots not covered by a broker binding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdmissionBindingDiagnostic {
    pub unbound_slots: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderRuntimeOutcome {
    pub provider: Arc<dyn LlmProvider>,
    pub admission_binding_diagnostic: AdmissionBindingDiagnostic,
}
