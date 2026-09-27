//! How one save reaches the store (#2218 cost): a full snapshot borrows the
//! live transcript instead of copying it, and gives it back even when the
//! save is cancelled mid-write.
use crate::application::sessions::dto::SaveMode;
use crate::application::sessions::ports::SessionStore;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::{PersistedSubagentRosterEntry, Session};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::workflow::WorkflowRunPersisted;

/// One save's write: a full snapshot, a clean delta from `watermark`, or
/// (`verified`) a delta the store checks against its file first.
pub(super) struct Write<'a> {
    pub mode: SaveMode,
    pub verified: bool,
    pub identity: &'a SessionIdentity,
    pub messages: &'a mut Vec<Message>,
    pub watermark: usize,
    pub workflow_run: Option<WorkflowRunPersisted>,
    pub roster: Option<Vec<PersistedSubagentRosterEntry>>,
}

pub(super) async fn write(store: &dyn SessionStore, write: Write<'_>) -> Result<(), DomainError> {
    let Write {
        mode,
        verified,
        identity,
        messages,
        watermark,
        workflow_run,
        roster,
    } = write;
    match (mode, verified) {
        (SaveMode::Full, _) => {
            let session = Session {
                key: identity.clone(),
                messages: Vec::new(),
                workflow_run,
                subagent_roster: roster.unwrap_or_default(),
            };
            let lent = Lent::new(messages, session);
            store.save(&lent.session).await
        }
        (SaveMode::CleanDelta, true) => {
            store
                .save_delta(identity, messages, watermark, workflow_run)
                .await
        }
        (SaveMode::CleanDelta, false) => {
            store
                .save_clean_delta(identity, messages, watermark, workflow_run)
                .await
        }
    }
}

/// The live transcript moved into a snapshot for the write, and moved back
/// when the write ends or is dropped.
struct Lent<'m> {
    home: &'m mut Vec<Message>,
    session: Session,
}

impl<'m> Lent<'m> {
    fn new(home: &'m mut Vec<Message>, mut session: Session) -> Self {
        std::mem::swap(home, &mut session.messages);
        Self { home, session }
    }
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        std::mem::swap(self.home, &mut self.session.messages);
    }
}
