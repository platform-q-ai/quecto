//! The application-owned state of the one active session of a harness loop
//! (#1968 R7a, D2 #1971): its typed identity and its live-conversation read
//! model. Composition creates exactly one per loop and hands it down as an
//! [`ActiveSessionHandle`]; the history and recovery use cases operate on
//! it directly, the loop publishes into it, and the interface only
//! invokes and presents.
//!
//! D5 extends this same object with persistence state (durable-prefix
//! latch, persisted watermark, killing-exit state); no second owner of
//! conversation or identity state may appear beside it.
use std::sync::Arc;

use super::conversation_ledger::{ConversationLedger, LedgerAdvance};
use super::ports::ContextSpillStore;
use crate::domain::message::Message;
use crate::domain::session_identity::{SessionIdentity, SpillId};

/// Shared, lock-guarded handle on the loop's active session.
pub type ActiveSessionHandle = Arc<tokio::sync::RwLock<ActiveSessionState>>;

pub struct ActiveSessionState {
    identity: SessionIdentity,
    conversation: ConversationLedger,
}

impl ActiveSessionState {
    /// The state of a loop opened on `identity`, with nothing published yet.
    pub fn new(identity: SessionIdentity) -> Self {
        Self {
            identity,
            conversation: ConversationLedger::default(),
        }
    }

    pub fn identity(&self) -> &SessionIdentity {
        &self.identity
    }

    pub fn conversation(&self) -> &ConversationLedger {
        &self.conversation
    }

    /// Publish the live transcript (see [`ConversationLedger::publish`]).
    pub fn publish(&mut self, messages: &[Message]) -> LedgerAdvance {
        self.conversation.publish(messages)
    }

    /// Record full copies (see [`ConversationLedger::record_full`]).
    pub fn record_full(&mut self, messages: &[Message]) -> LedgerAdvance {
        self.conversation.record_full(messages)
    }

    /// Attach the retention store of the current identity (see
    /// [`ConversationLedger::set_spill_store`]).
    pub fn set_spill_store(&mut self, spill_store: Option<Arc<dyn ContextSpillStore>>) {
        self.conversation.set_spill_store(spill_store);
    }

    /// Drop the transcript and open a new epoch (see
    /// [`ConversationLedger::clear`]).
    pub fn clear(&mut self) -> LedgerAdvance {
        self.conversation.clear()
    }

    /// Switch the session this state stands for: its identity and the
    /// retention store paired with it, in one step, right after
    /// [`Self::clear`] (which already invalidated every deferred recall).
    pub fn switch_identity(
        &mut self,
        identity: SessionIdentity,
        spill_store: Option<Arc<dyn ContextSpillStore>>,
    ) {
        self.identity = identity;
        self.conversation
            .replace_spill_store_after_clear(spill_store);
    }

    /// Prepare a recovery lookup. Collapsed messages that carry a spill id
    /// are returned as a deferred recall so callers do not hold the lock
    /// across retention I/O.
    pub fn resolve_for_recovery(&self, message_id: &str) -> RecoveryResolution {
        let Some(msg) = self.conversation.lookup(message_id) else {
            return RecoveryResolution::NotFound;
        };
        if !msg.is_collapsed {
            return RecoveryResolution::Found(msg.clone());
        }
        let Some(spill_id) = msg.spill_id.clone() else {
            return RecoveryResolution::Found(msg.clone());
        };
        let Some(spill_store) = self.conversation.spill_store().cloned() else {
            return RecoveryResolution::Found(msg.clone());
        };
        RecoveryResolution::Recall {
            stub: msg.clone(),
            spill_store,
            identity: self.identity.clone(),
            spill_id,
            generation: self.conversation.generation(),
        }
    }

    /// Whether a completed deferred recall still describes this state: same
    /// generation and identity, and the message is still the collapsed
    /// stub it was recalled for.
    pub fn recall_is_current(&self, recall: &RecallIdentity) -> bool {
        self.conversation.generation() == recall.generation
            && self.identity == recall.identity
            && self
                .conversation
                .lookup(&recall.message_id)
                .is_some_and(|m| {
                    m.is_collapsed && m.spill_id.as_deref() == Some(recall.spill_id.as_str())
                })
    }
}

impl std::fmt::Debug for ActiveSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveSessionState")
            .field("identity", &self.identity)
            .field("live_messages", &self.conversation.live_messages().len())
            .field("epoch", &self.conversation.epoch())
            .field("rev", &self.conversation.rev())
            .finish_non_exhaustive()
    }
}

/// A recovery lookup prepared under the lock: found outright, deferred to
/// the retention store, or absent.
pub enum RecoveryResolution {
    Found(Message),
    Recall {
        stub: Message,
        spill_store: Arc<dyn ContextSpillStore>,
        identity: SessionIdentity,
        spill_id: String,
        generation: u64,
    },
    NotFound,
}

/// What a deferred recall was prepared against, so a completed read can be
/// rejected if a lifecycle operation replaced the history meanwhile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallIdentity {
    pub message_id: String,
    pub identity: SessionIdentity,
    pub spill_id: String,
    pub generation: u64,
}

/// A recovery lookup completed outside the lock.
pub struct ResolvedRecovery {
    pub message: Option<Message>,
    pub recalled: Option<RecallIdentity>,
}

impl RecoveryResolution {
    /// Finish the best-effort lookup outside the lock. Spill misses and
    /// errors fall back to the live collapsed stub. A deferred read carries
    /// its captured identity so the caller can reject it if a concurrent
    /// lifecycle operation replaced the history.
    pub async fn into_message(self) -> ResolvedRecovery {
        match self {
            Self::Found(message) => ResolvedRecovery {
                message: Some(message),
                recalled: None,
            },
            Self::Recall {
                mut stub,
                spill_store,
                identity,
                spill_id,
                generation,
            } => {
                let recalled = RecallIdentity {
                    message_id: stub.id().to_string(),
                    identity: identity.clone(),
                    spill_id: spill_id.clone(),
                    generation,
                };
                if let Ok(Some(entry)) = spill_store
                    .recall(&identity, &SpillId::new(spill_id.as_str()))
                    .await
                {
                    stub.content = entry.content;
                    stub.is_collapsed = false;
                    stub.spill_id = None;
                }
                ResolvedRecovery {
                    message: Some(stub),
                    recalled: Some(recalled),
                }
            }
            Self::NotFound => ResolvedRecovery {
                message: None,
                recalled: None,
            },
        }
    }
}

#[cfg(test)]
#[path = "active_session_tests.rs"]
mod tests;
