//! Narrow outbound ports of the admission authority.
use crate::domain::admission::value_objects::inference_admission::AdmissionLedger;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    /// The ledger could not be made durable (file and directory).
    Unavailable(String),
}

/// Durable ledger sink. `persist` must return only after the ledger is durable
/// against process and power loss; a warning-only sync is not sufficient.
pub trait AdmissionJournal {
    fn persist(&mut self, ledger: &AdmissionLedger) -> Result<(), JournalError>;
}

/// The one journal the authority's durability use cases share, fixed for the
/// broker's life (#1910): composition builds it and hands it to each.
pub type SharedJournal = std::sync::Arc<std::sync::Mutex<dyn AdmissionJournal + Send>>;

/// Source of opaque capability material for issued scopes.
pub trait AdmissionSecretSource {
    fn mint(&mut self) -> String;
}
