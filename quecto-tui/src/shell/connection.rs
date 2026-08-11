//! Master-connection feed task seam (#1462, epic #1467).
//!
//! Phase 1 of the multi-session TUI: the master [`Client`] moves behind a
//! [`Connection`] feed task, modelled on the sub-agent feed task pattern
//! (`agents/controller_subagent_feed.rs`): a tokio task owns the socket's
//! event stream, commands ride the client's existing FIFO writer mpsc
//! (whose task owns the socket's write half), and events are forwarded into
//! the shared fan-in channel keyed by [`Source`]. The event loop's select
//! arm count becomes independent of connection count, and stream close is an
//! explicit [`Source::Closed`] sentinel instead of `None`-from-recv.

use crate::protocol::client::{Client, ClientError, Command, CommandSender, Event};
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
pub(crate) type SourcedEvent = (Source, Option<Event>);

/// A master connection behind a feed task: the feed task owns the [`Client`]
/// (and with it the socket's event stream), forwards events into the shared
/// fan-in tagged `Source::Tab(tab)`, and emits `Source::Closed(tab)` when
/// the stream closes. Callers hold only this handle.
///
/// Commands ride the client's existing ordered writer mpsc — the "small
/// cmd_tx" of the sub-agent feed pattern — whose task owns the socket's
/// write half. Adding a second command queue in front of it would change
/// the #1238 backpressure/reservation semantics and the failure surface of
/// `try_send`; the seam deliberately reuses the queue that already provides
/// FIFO order and a non-blocking enqueue.
pub(crate) struct Connection {
    sender: CommandSender,
    /// Per-connection ADR-0008 negotiation outcome (#1462 scope 4), copied
    /// from the [`Client`]'s connect-time framing.
    speaks_frames: bool,
    /// Shared handle to the reader's oversized-drop counter (#1047), kept
    /// observable after the client moves into the feed task.
    dropped_oversized: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Connection {
    /// Move `client` behind the feed task, forwarding its events into
    /// `event_tx` keyed by `tab` and closing with a `Source::Closed(tab)`
    /// sentinel. The negotiation outcome (`speaks_frames`) is read from the
    /// client itself — per-connection state, not a caller-supplied flag.
    ///
    /// Outside a tokio runtime (sync unit tests building an `App` around a
    /// disconnected stub client) no task can be spawned; the client is
    /// dropped and the connection only carries the command sender, which is
    /// exactly the pre-seam behaviour those tests exercised.
    pub(crate) fn spawn(client: Client, tab: TabId, event_tx: mpsc::Sender<SourcedEvent>) -> Self {
        let sender = client.clone_sender();
        let speaks_frames = client.speaks_frames();
        let dropped_oversized = client.dropped_oversized_handle();
        if tokio::runtime::Handle::try_current().is_ok() {
            let mut client = client;
            tokio::spawn(async move {
                loop {
                    match client.recv().await {
                        Some(ev) => {
                            if event_tx.send((Source::Tab(tab), Some(ev))).await.is_err() {
                                return; // App gone — nothing left to feed.
                            }
                        }
                        None => {
                            // Stream closed: the explicit sentinel replaces
                            // `None`-from-recv on a dedicated select arm.
                            let _ = event_tx.send((Source::Closed(tab), None)).await;
                            return;
                        }
                    }
                }
            });
        }
        Self {
            sender,
            speaks_frames,
            dropped_oversized,
        }
    }

    /// Enqueue a command onto the connection's ordered writer channel
    /// without blocking (FIFO order; #1238 backpressure semantics of
    /// [`CommandSender::try_send`] apply unchanged).
    pub(crate) fn try_send(&self, cmd: &Command) -> Result<(), ClientError> {
        self.sender.try_send(cmd)
    }

    /// A cloneable command sender for spawned tasks (the routed sub-agent
    /// inspection feeds) — same ordered channel as [`Self::try_send`].
    pub(crate) fn clone_sender(&self) -> CommandSender {
        self.sender.clone()
    }

    /// Per-connection ADR-0008 negotiation outcome: whether this connection
    /// speaks length-prefixed frames (vs legacy NDJSON).
    // Consumed by the seam's contract tests today; production reads arrive
    // with the ADR-0008 part-3 handshake and the N>1 tabs of epic #1467.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn speaks_frames(&self) -> bool {
        self.speaks_frames
    }

    /// How many event lines this connection's reader dropped for exceeding
    /// the line cap (#1047). The UI polls this to surface the loss.
    pub(crate) fn dropped_oversized_events(&self) -> u64 {
        self.dropped_oversized
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Test-only: a connection whose writer channel is already closed, so
    /// `try_send` deterministically fails with `Disconnected` — the seam
    /// replacement for swapping in `Client::disconnected_for_tests()`.
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn disconnected_for_tests() -> Self {
        let client = Client::disconnected_for_tests();
        Self {
            sender: client.clone_sender(),
            speaks_frames: client.speaks_frames(),
            dropped_oversized: client.dropped_oversized_handle(),
        }
    }

    /// Test-only: simulate the reader recording `n` oversized-line drops.
    #[cfg(test)]
    pub(crate) fn record_dropped_oversized_for_tests(&self, n: u64) {
        self.dropped_oversized
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod connection_tests;
