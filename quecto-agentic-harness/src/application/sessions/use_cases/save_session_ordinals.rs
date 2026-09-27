//! The durable ordinals one save transaction stamps (#2218): assigned to
//! the live transcript before the store write and withdrawn unless the
//! write commits — on a store error and on a save cancelled mid-write — so
//! a live message never serves an ordinal the store does not hold. A
//! supervisor delivers any ordinal it reads; one lost to a failed write
//! would be re-issued after a restart and hide the later message.
use std::collections::HashSet;

use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::SaveSessionError;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::assign_missing_ordinals;

/// Set by a save that ended without committing — failed or cancelled after
/// its write may have landed — so the next save verifies the file (#2218).
#[derive(Default)]
pub(super) struct Unsettled(std::sync::atomic::AtomicBool);

impl Unsettled {
    pub(super) fn take(&self) -> bool {
        self.0.swap(false, std::sync::atomic::Ordering::AcqRel)
    }
}

/// The (prompt-stripped) live transcript with this save's ordinals
/// stamped; they are withdrawn on drop unless [`Stamped::commit`] ran, and
/// the save is then marked [`Unsettled`].
pub(super) struct Stamped<'m> {
    messages: &'m mut Vec<Message>,
    ids: HashSet<uuid::Uuid>,
    unsettled: &'m Unsettled,
    committed: bool,
}

impl<'m> Stamped<'m> {
    /// Number every unnumbered message, above every earlier ordinal.
    pub(super) fn stamp(messages: &'m mut Vec<Message>, unsettled: &'m Unsettled) -> Self {
        let earlier_max = messages.iter().filter_map(|m| m.ordinal).max();
        let ids: HashSet<uuid::Uuid> = messages
            .iter()
            .filter(|message| message.ordinal.is_none())
            .map(Message::id)
            .collect();
        assign_missing_ordinals(messages);
        debug_assert!(
            {
                let fresh: Vec<u64> = messages
                    .iter()
                    .filter(|m| ids.contains(&m.id()))
                    .filter_map(|m| m.ordinal)
                    .collect();
                let distinct: HashSet<u64> = fresh.iter().copied().collect();
                fresh.len() == ids.len()
                    && distinct.len() == fresh.len()
                    && fresh.iter().all(|o| earlier_max.is_none_or(|max| *o > max))
            },
            "a save numbers every new message once, above every earlier ordinal"
        );
        Self {
            messages,
            ids,
            unsettled,
            committed: false,
        }
    }

    pub(super) fn messages(&mut self) -> &mut Vec<Message> {
        self.messages
    }

    /// The store holds the transcript: its ordinals stay.
    pub(super) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for Stamped<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.unsettled
            .0
            .store(true, std::sync::atomic::Ordering::Release);
        let mut withdrawn = 0usize;
        for message in self
            .messages
            .iter_mut()
            .filter(|message| self.ids.contains(&message.id()))
        {
            message.ordinal = None;
            withdrawn += 1;
        }
        debug_assert_eq!(
            withdrawn,
            self.ids.len(),
            "every stamped ordinal is withdrawn"
        );
    }
}

/// Keep the stamped ordinals when the write committed. On any store error
/// they are withdrawn and the prefix is latched dirty (#2218): a write may
/// have landed before it failed (its fsync, say), so the next save verifies
/// the file instead of appending from a stale watermark.
pub(super) async fn settle(
    state: &ActiveSessionHandle,
    result: Result<(), DomainError>,
    stamped: Stamped<'_>,
) -> Result<(), SaveSessionError> {
    match result {
        Ok(()) => {
            stamped.commit();
            Ok(())
        }
        Err(error) => {
            drop(stamped);
            state.write().await.latch_durable_prefix_dirty();
            Err(SaveSessionError::Store(error))
        }
    }
}
