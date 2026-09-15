//! Raw export source of the active session (#1859; D4 #1974 retires this
//! into the export use case): what one consistent read of the session
//! yields for a raw export.
use super::*;

/// Everything a raw export records, read under one lock.
pub(crate) struct ExportSource {
    pub epoch: u64,
    pub revision: u64,
    pub messages: Vec<Message>,
    pub spill_store: Option<Arc<dyn ContextSpillStore>>,
    pub identity: crate::domain::session_identity::SessionIdentity,
}

pub(crate) fn export_source(state: &ActiveSessionState) -> ExportSource {
    let ledger = state.conversation();
    ExportSource {
        epoch: ledger.epoch(),
        revision: ledger.rev(),
        messages: ledger.retained_messages(),
        spill_store: ledger.spill_store().cloned(),
        identity: state.identity().clone(),
    }
}
