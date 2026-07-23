use crate::infrastructure::client::Command;
use crate::interface::ledger_sync::LedgerTranscript;
use std::time::Instant;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedAuthority {
    LegacySelected,
    WarmSync,
    SyncedAuthoritative,
}

pub(crate) struct FeedState {
    pub(crate) cmd_tx: mpsc::Sender<Command>,
    pub(crate) handle: tokio::task::JoinHandle<()>,
    pub(crate) epoch: u64,
    pub(crate) rev: u64,
    pub(crate) last_fresh_at: Option<Instant>,
    pub(crate) supports_sync: bool,
    pub(crate) pending_rev: Option<u64>,
    pub(crate) transcript: LedgerTranscript,
    pub(crate) authority: FeedAuthority,
}
