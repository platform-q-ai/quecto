//! Inspect a sub-agent that has ended (#2192): why it ended, and what it
//! did. A live child answers for itself over its socket; an ended one no
//! longer can, so its parent reads what it left behind — the crash record
//! its panic hook wrote, and the transcript it persisted up to its end.
use std::sync::Arc;

use crate::application::subagents::ports::EndedChildRecords;
use crate::domain::agents::value_objects::child_end::ChildOrigin;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::crash_record::CrashRecord;
use crate::domain::ids::AgentUuid;
use crate::domain::sessions::entities::session_identity::SessionIdentity;

/// A window of an ended child's persisted transcript. Paged by the durable
/// ordinals persistence assigns, not by message ids: those are minted anew
/// on every load, so a cursor from one read would name nothing in the next.
#[derive(Debug)]
pub struct EndedTranscriptPage {
    pub messages: Vec<Message>,
    /// The ordinal to pass as `before` for the older window, when there is
    /// one and its first message carries an ordinal.
    pub before: Option<u64>,
    pub has_more_before: bool,
    /// Messages older than the transcript's newest part were not read.
    pub older_omitted: bool,
}

/// Why an ended child's transcript cannot be served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndedTranscriptError {
    /// Nothing this harness can read holds it.
    NotHere(String),
    /// The store holds it but could not read it.
    Unreadable(String),
    /// The `before` ordinal names no message of it.
    UnknownCursor(u64),
}

impl std::fmt::Display for EndedTranscriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotHere(reason) | Self::Unreadable(reason) => f.write_str(reason),
            Self::UnknownCursor(ordinal) => {
                write!(
                    f,
                    "no message with ordinal {ordinal} in its persisted transcript"
                )
            }
        }
    }
}

pub struct InspectEndedChild {
    records: Arc<dyn EndedChildRecords>,
    /// Where `records` reads sessions from, for the "not here" reason.
    store: String,
}

impl InspectEndedChild {
    /// The most messages one page of an ended child's transcript holds:
    /// more than one bounded answer can carry, so a larger `count` only
    /// copied messages to discard them (#2192 review).
    pub const MAX_PAGE_MESSAGES: usize = 256;

    pub fn new(records: Arc<dyn EndedChildRecords>, store: impl Into<String>) -> Self {
        Self {
            records,
            store: store.into(),
        }
    }

    /// The session a launched child runs as: `cli:<uuid>` (its `-s`).
    pub fn child_session(child: &AgentUuid) -> Option<SessionIdentity> {
        crate::domain::agents::services::child_session::child_session_identity(child).ok()
    }

    /// The crash record the child left, if it left one. A child this
    /// harness launched is asked for by its own process (`process`, its
    /// process id, when known). Any other row's uuid is another agent's word (#2192 review):
    /// a record is read for it only under a uuid of the form this harness
    /// mints — never under an arbitrary name — and never as a known
    /// process's, so nothing it says can be believed.
    pub async fn crash(
        &self,
        child: &AgentUuid,
        origin: ChildOrigin,
        process: Option<u32>,
    ) -> Option<CrashRecord> {
        let writer = match (origin, child.is_canonical()) {
            (ChildOrigin::Launched, _) => process,
            (ChildOrigin::Reported | ChildOrigin::Unverified, true) => None,
            (ChildOrigin::Reported | ChildOrigin::Unverified, false) => return None,
        };
        let identity = Self::child_session(child)?;
        self.records.crash(&identity, writer).await
    }

    /// Whether `transcript` would serve the child's transcript: a child this
    /// harness launched, whose session this harness's store holds and can
    /// read (#2192 review: an exit note offers it only then).
    pub async fn has_transcript(&self, child: &AgentUuid, origin: ChildOrigin) -> bool {
        let identity = match (origin, Self::child_session(child)) {
            (ChildOrigin::Launched, Some(identity)) => identity,
            (ChildOrigin::Launched, None)
            | (ChildOrigin::Reported | ChildOrigin::Unverified, _) => return false,
        };
        self.records.has_transcript(&identity).await
    }

    /// The newest `count` visible messages of the child's persisted
    /// transcript before the message whose ordinal is `before` (the newest
    /// ones without it).
    ///
    /// Only a child this harness launched has its transcript read (#2192
    /// review): a reported descendant's uuid is its reporter's word, and
    /// `named_cli` of it could name any session (`secret-plan`), so a child
    /// could make its parent read another conversation. No session records
    /// who launched it, so there is nothing else to check it against.
    pub async fn transcript(
        &self,
        child: &AgentUuid,
        origin: ChildOrigin,
        count: usize,
        before: Option<u64>,
    ) -> Result<EndedTranscriptPage, EndedTranscriptError> {
        assert!(count > 0, "a page holds at least one message");
        let count = count.min(Self::MAX_PAGE_MESSAGES);
        match origin {
            ChildOrigin::Launched => {}
            ChildOrigin::Reported | ChildOrigin::Unverified => {
                return Err(EndedTranscriptError::NotHere(format!(
                    "this harness did not launch '{child}' itself (another agent reported it), \
                     so no transcript is read under its name"
                )));
            }
        }
        let Some(identity) = Self::child_session(child) else {
            return Err(EndedTranscriptError::NotHere(format!(
                "'{child}' is not a launched child's identity, so it has no session to read"
            )));
        };
        let transcript = match self.records.transcript(&identity).await {
            Ok(Some(transcript)) => transcript,
            Ok(None) => {
                return Err(EndedTranscriptError::NotHere(format!(
                    "no transcript for session '{}' is in {} (a child that ran in a container \
                     whose session store is not shared with this harness keeps it there)",
                    identity.runtime_key(),
                    self.store
                )));
            }
            Err(error) => {
                return Err(EndedTranscriptError::Unreadable(format!(
                    "its transcript in {} could not be read: {error}",
                    self.store
                )));
            }
        };
        // A persisted transcript holds no injected system prompt: every
        // message of it is the child's visible conversation.
        let visible: Vec<&Message> = transcript.messages.iter().collect();
        // Durable ordinals to page by, else (a transcript written before
        // them, or its newest part) positions, which an ended child keeps.
        let numbered = visible.iter().all(|message| message.ordinal.is_some());
        let ordinal = |position: usize, message: &Message| match numbered {
            true => message.ordinal,
            false => Some(position as u64 + 1),
        };
        let end = match before {
            Some(cursor) => visible
                .iter()
                .enumerate()
                .position(|(position, message)| ordinal(position, message) == Some(cursor))
                .ok_or(EndedTranscriptError::UnknownCursor(cursor))?,
            None => visible.len(),
        };
        let start = end.saturating_sub(count);
        debug_assert!(start <= end && end <= visible.len());
        let has_more_before = start > 0;
        let messages = (start..end)
            .map(|position| {
                let mut message = visible[position].clone();
                message.ordinal = ordinal(position, visible[position]);
                message
            })
            .collect();
        Ok(EndedTranscriptPage {
            before: has_more_before
                .then(|| ordinal(start, visible[start]))
                .flatten(),
            has_more_before,
            older_omitted: transcript.older_omitted,
            messages,
        })
    }
}

impl std::fmt::Debug for InspectEndedChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectEndedChild")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "inspect_ended_child_tests.rs"]
mod tests;
