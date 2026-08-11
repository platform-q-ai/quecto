//! Master-connection feed task seam (#1462, epic #1467).
//!
//! Phase 1 of the multi-session TUI: the master [`Client`] moves behind a
//! [`Connection`] feed task, modelled on the sub-agent feed task pattern
//! (`agents/controller_subagent_feed.rs`): a tokio task owns the socket,
//! receives commands on a small mpsc, and forwards events into the shared
//! fan-in channel keyed by [`Source`]. The event loop's select arm count
//! becomes independent of connection count, and stream close is an explicit
//! [`Source::Closed`] sentinel instead of `None`-from-recv.
//!
//! NOTE (#1462 RED): this module is currently a non-functional stub so the
//! seam's contract tests compile and fail before the implementation lands.

use crate::protocol::client::{Client, ClientError, Command, Event};
use tokio::sync::mpsc;

/// Stable identity of one TUI tab (one master connection). Phase 1 has
/// exactly one: [`TabId::MASTER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TabId(pub(crate) u32);

impl TabId {
    /// The single tab of the N=1 phase.
    pub(crate) const MASTER: TabId = TabId(0);
}

/// Fan-in key for events drained by the app event loop. Widens the previous
/// `String` (sub-agent id) key so master-connection events and sub-agent
/// events share ONE channel, and stream close is an explicit sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// The tab's master connection itself.
    Tab(TabId),
    /// A sub-agent feed belonging to the tab.
    Subagent(TabId, String),
    /// The tab's master connection stream closed (replaces `None`-from-recv).
    Closed(TabId),
}

/// One item on the shared fan-in channel. `Source::Closed` is the only
/// source delivered without an event payload.
// RED (#1462): only referenced from tests until the event loop drains the
// widened fan-in; the allow is removed when GREEN wires it in.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) type SourcedEvent = (Source, Option<Event>);

/// A master connection behind a feed task: the task owns the [`Client`],
/// receives commands on `cmd_tx`, forwards events into the shared fan-in
/// tagged `Source::Tab(tab)`, and emits `Source::Closed(tab)` when the
/// stream closes.
// RED (#1462): constructed only by tests until `run_tui`/`App` own a
// `Connection`; the allows are removed when GREEN wires it in.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Connection {
    cmd_tx: mpsc::Sender<Command>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Connection {
    /// Spawn the feed task for `client`, forwarding its events into
    /// `event_tx` keyed by `tab`. `speaks_frames` records the per-connection
    /// ADR-0008 protocol negotiation outcome.
    ///
    /// RED stub (#1462): drops the client and spawns nothing.
    pub(crate) fn spawn(
        client: Client,
        tab: TabId,
        event_tx: mpsc::Sender<SourcedEvent>,
        speaks_frames: bool,
    ) -> Self {
        let _ = (client, tab, event_tx, speaks_frames);
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        Self { cmd_tx }
    }

    /// Enqueue a command onto the feed task without blocking (FIFO order).
    ///
    /// RED stub (#1462): always reports disconnected.
    pub(crate) fn try_send(&self, cmd: &Command) -> Result<(), ClientError> {
        let _ = (cmd, &self.cmd_tx);
        Err(ClientError::Disconnected)
    }

    /// Per-connection ADR-0008 negotiation outcome: whether this connection
    /// speaks length-prefixed frames (vs legacy NDJSON).
    ///
    /// RED stub (#1462): unimplemented.
    pub(crate) fn speaks_frames(&self) -> bool {
        unimplemented!("#1462: per-connection protocol negotiation not yet implemented")
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod connection_tests;
