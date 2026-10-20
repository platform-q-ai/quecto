//! What an ended sub-agent left behind (#2192): its crash record and its
//! persisted transcript.

use super::PortFuture;

/// What an ended sub-agent left behind (#2192), read by its parent once
/// the child can no longer answer for itself: the crash record its panic
/// hook wrote before it aborted, and the transcript it persisted.
pub trait EndedChildRecords: Send + Sync {
    /// The crash record of the child running as `child`, as the process
    /// `writer` left it (#2192 review): with a pid, only a record that
    /// process wrote — another's, however new, never stands in for it;
    /// with none, the newest of any. A missing or unreadable record is
    /// `None`: it explains a death when present and is never required.
    fn crash<'a>(
        &'a self,
        child: &'a crate::domain::sessions::entities::session_identity::SessionIdentity,
        writer: Option<u32>,
    ) -> PortFuture<'a, Option<crate::domain::agents::value_objects::crash_record::CrashRecord>>;

    /// The persisted transcript of the child running as `child`: `None`
    /// when this harness's store holds no session of that name.
    fn transcript<'a>(
        &'a self,
        child: &'a crate::domain::sessions::entities::session_identity::SessionIdentity,
    ) -> PortFuture<'a, Result<Option<EndedTranscript>, crate::domain::shared::error::DomainError>>;

    /// Whether a transcript of the child running as `child` is there to be
    /// read — cheaply, where the adapter can tell without reading it
    /// (#2192 review). By default, by reading it.
    fn has_transcript<'a>(
        &'a self,
        child: &'a crate::domain::sessions::entities::session_identity::SessionIdentity,
    ) -> PortFuture<'a, bool> {
        Box::pin(async move { matches!(self.transcript(child).await, Ok(Some(_))) })
    }
}

/// An ended child's persisted transcript, as far as it could be read. It no
/// longer changes, so readers share it rather than copy it.
#[derive(Debug, Clone)]
pub struct EndedTranscript {
    pub messages: std::sync::Arc<Vec<crate::domain::conversation::value_objects::message::Message>>,
    /// Only the newest part was read: older messages exist but are omitted.
    pub older_omitted: bool,
}
