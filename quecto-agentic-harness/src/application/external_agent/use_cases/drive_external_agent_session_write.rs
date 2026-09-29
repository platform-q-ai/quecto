//! The member session's writes to its agent (#2287 review 5): the write
//! gate, a follow-up started under it, and a user turn queued in two
//! steps, undone or kept when its caller is dropped before it is queued.

use super::DriveExternalAgentSession;
use crate::application::external_agent::dto::{
    PromptAccepted, SessionRecord, SessionRefusal, SessionStep,
};

impl DriveExternalAgentSession {
    /// Take the write gate. A follow-up whose turn has begun but whose
    /// user turn a dropped holder left unqueued is written first.
    pub(super) async fn gate(&self) -> tokio::sync::MutexGuard<'_, ()> {
        let writes = self.writes.lock().await;
        self.start_follow_up().await;
        writes
    }

    /// Write the follow-up whose turn has begun, if any; the caller holds
    /// the write gate. Dropped before it is queued, it is kept for the
    /// next holder.
    pub(super) async fn start_follow_up(&self) {
        let Some((turn, text)) = self.unwritten_slot().take() else {
            return;
        };
        let accepted = PromptAccepted::Started { turn };
        if let Err(refusal) = self.write(accepted, &text, Undo::Keep { turn }).await {
            self.record(SessionRecord::FollowUpFailed {
                turn,
                bytes: text.len(),
                refusal: refusal.kind(),
            });
            self.core()
                .surface(SessionStep::FollowUpFailed { turn, refusal });
        }
    }

    /// Write `accepted`'s text to the agent, unless the member ends first.
    /// Once queued it is part of the conversation, and owed a result;
    /// `undo` says what a caller dropped before then leaves behind.
    pub(super) async fn write(
        &self,
        accepted: PromptAccepted,
        text: &str,
        undo: Undo,
    ) -> Result<(), SessionRefusal> {
        let process = self.slot().clone().ok_or(SessionRefusal::Ended)?;
        let mut unqueued = Unqueued {
            session: self,
            text,
            undo: Some(undo),
        };
        let queued = tokio::select! {
            biased;
            () = self.until_ended() => Err(SessionRefusal::Ended),
            queued = process.queue_user_turn(text) => queued.map_err(SessionRefusal::Input),
        };
        unqueued.undo = None;
        let queued = match queued {
            Ok(queued) => queued,
            Err(refusal) => {
                self.core().write_failed(accepted);
                return Err(refusal);
            }
        };
        let id = queued.id.clone();
        self.core().written(queued.id, text);
        let written = tokio::select! {
            biased;
            () = self.until_ended() => Err(SessionRefusal::Ended),
            written = queued.written => written.map_err(SessionRefusal::Input),
        };
        match written {
            Ok(()) => Ok(()),
            Err(refusal) => {
                let mut core = self.core();
                core.not_written(&id);
                core.write_failed(accepted);
                Err(refusal)
            }
        }
    }
}

/// What a write whose caller is dropped before its user turn is queued
/// leaves behind.
#[derive(Clone, Copy)]
pub(super) enum Undo {
    /// The prompt's admission is undone: a turn it began never started.
    Admission(PromptAccepted),
    /// Follow-up turn `turn` is kept, for the next holder of the gate.
    Keep { turn: u64 },
}

/// Armed until a write's user turn is queued or refused: dropped armed,
/// the write's caller was, and its [`Undo`] is done.
struct Unqueued<'a> {
    session: &'a DriveExternalAgentSession,
    text: &'a str,
    undo: Option<Undo>,
}

impl Drop for Unqueued<'_> {
    fn drop(&mut self) {
        match self.undo.take() {
            Some(Undo::Admission(accepted)) => self.session.core().write_failed(accepted),
            Some(Undo::Keep { turn }) => {
                *self.session.unwritten_slot() = Some((turn, self.text.to_string()));
            }
            None => {}
        }
    }
}
