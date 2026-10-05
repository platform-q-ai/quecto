//! Runtime ownership for direct child feeds (#1222 / #1257 Phase 4).
//!
//! Kept separate from feed sync policy and view state so pure synchronization
//! state has no Tokio channel or task handle, and view state does not own the
//! connect-task lifecycle type definition.

use crate::protocol::client::Command;
use tokio::sync::mpsc;

/// Runtime ownership for a direct child feed. Kept separate from feed sync
/// policy so pure synchronization state has no Tokio channel or task handle.
pub(crate) struct FeedRuntime {
    pub(crate) cmd_tx: mpsc::Sender<Command>,
    pub(crate) handle: tokio::task::JoinHandle<()>,
    pub(crate) inspection_only: bool,
}
