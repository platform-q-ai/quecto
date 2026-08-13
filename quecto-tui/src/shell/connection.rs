//! Master-connection feed task seam (#1462, epic #1467).
//!
//! Phase 1 of the multi-session TUI: the master [`Client`] moves behind a
//! [`Connection`] feed task, modelled on the sub-agent feed task pattern
//! (`agents/controller_subagent_feed.rs`): a tokio task owns the socket's
//! event stream, commands ride the client's existing FIFO writer mpsc
//! (whose task owns the socket's write half), and events are forwarded into
//! the shared fan-in channel keyed by [`SourcedEvent`]. The event loop's select
//! arm count becomes independent of connection count, and stream close is an
//! explicit [`SourcedEvent::Closed`] sentinel instead of `None`-from-recv.

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

/// One item on a fan-in channel drained by the app event loop. The payload
/// lives inside the variant, so an event-less `Tab`/`Subagent` item or a
/// payload-carrying `Closed` sentinel is unrepresentable — consumers need no
/// dead arms for states no producer constructs (#1470 review).
#[derive(Debug, Clone)]
pub(crate) enum SourcedEvent {
    /// An event from the tab's master connection.
    Tab(TabId, Event),
    /// An event from a sub-agent feed belonging to the tab.
    Subagent(TabId, String, Event),
    /// The tab's master connection stream closed (replaces `None`-from-recv).
    Closed(TabId),
}

impl SourcedEvent {
    /// The tab this item belongs to, regardless of variant.
    pub(crate) fn tab(&self) -> TabId {
        match self {
            SourcedEvent::Tab(tab, _)
            | SourcedEvent::Subagent(tab, _, _)
            | SourcedEvent::Closed(tab) => *tab,
        }
    }
}

/// A master connection behind a feed task: the feed task owns the [`Client`]
/// (and with it the socket's event stream), forwards events into the shared
/// fan-in tagged `SourcedEvent::Tab(tab, ..)`, and emits `SourcedEvent::Closed(tab)` when
/// the stream closes. Callers hold only this handle.
///
/// Commands ride the client's existing ordered writer mpsc — the "small
/// cmd_tx" of the sub-agent feed pattern — whose task owns the socket's
/// write half. Adding a second command queue in front of it would change
/// the #1238 backpressure/reservation semantics and the failure surface of
/// `try_send`; the seam deliberately reuses the queue that already provides
/// FIFO order and a non-blocking enqueue.
pub(crate) struct Connection {
    /// The tab this connection belongs to. Every correlation id the tab
    /// mints is namespaced `tab{N}:` from this id (#1463), so broadcast
    /// responses can never match another tab's pending latches.
    // Production reads arrive with the cluster-2 id minting (#1463); until
    // then only the test re-key hook touches it.
    #[cfg_attr(not(any(test, feature = "test-harness")), allow(dead_code))]
    tab: TabId,
    sender: CommandSender,
    /// Per-connection ADR-0008 negotiation outcome (#1462 scope 4), copied
    /// from the [`Client`]'s connect-time framing.
    speaks_frames: bool,
    /// Shared handle to the reader's oversized-drop counter (#1047), kept
    /// observable after the client moves into the feed task.
    dropped_oversized: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// The feed task owning the client, when one was spawned. Held so the
    /// harness can abort it when it swaps this connection out — otherwise
    /// the orphaned task keeps the real socket and injects a spurious
    /// `Closed` sentinel into the fan-in later (#1470 review).
    // Read only by the harness's `abort_feed`; in production the task runs
    // for the connection's whole life and exits with the process.
    #[cfg_attr(not(any(test, feature = "test-harness")), allow(dead_code))]
    feed_task: Option<tokio::task::JoinHandle<()>>,
}

impl Connection {
    /// Move `client` behind the feed task, forwarding its events into
    /// `event_tx` keyed by `tab` and closing with a `SourcedEvent::Closed(tab)`
    /// sentinel. The negotiation outcome (`speaks_frames`) is read from the
    /// client itself — per-connection state, not a caller-supplied flag.
    ///
    /// In TEST builds only, outside a tokio runtime (sync unit tests
    /// building an `App` around a disconnected stub client) no task can be
    /// spawned; the client is dropped and the connection only carries the
    /// command sender — exactly the pre-seam behaviour those tests
    /// exercised. Production builds always spawn, and panic loudly if no
    /// runtime is present.
    pub(crate) fn spawn(client: Client, tab: TabId, event_tx: mpsc::Sender<SourcedEvent>) -> Self {
        let sender = client.clone_sender();
        let speaks_frames = client.speaks_frames();
        let dropped_oversized = client.dropped_oversized_handle();
        // The no-runtime fallback exists ONLY for test builds: sync unit
        // tests build an `App` around a disconnected stub client with no
        // tokio runtime. In production builds we spawn unconditionally, so a
        // future caller outside a runtime fails loudly (`tokio::spawn`
        // panics) instead of silently dropping the client and freezing the
        // tab with no `SourcedEvent::Closed` sentinel (#1047 class, PR review).
        #[cfg(any(test, feature = "test-harness"))]
        let spawn_feed = tokio::runtime::Handle::try_current().is_ok();
        #[cfg(not(any(test, feature = "test-harness")))]
        let spawn_feed = true;
        let feed_task = if spawn_feed {
            let mut client = client;
            Some(tokio::spawn(async move {
                loop {
                    match client.recv().await {
                        Some(ev) => {
                            if event_tx.send(SourcedEvent::Tab(tab, ev)).await.is_err() {
                                return; // App gone — nothing left to feed.
                            }
                        }
                        None => {
                            // Stream closed: the explicit sentinel replaces
                            // `None`-from-recv on a dedicated select arm.
                            let _ = event_tx.send(SourcedEvent::Closed(tab)).await;
                            return;
                        }
                    }
                }
            }))
        } else {
            None
        };
        Self {
            tab,
            sender,
            speaks_frames,
            dropped_oversized,
            feed_task,
        }
    }

    /// The tab this connection belongs to.
    pub(crate) fn tab(&self) -> TabId {
        self.tab
    }

    /// Test-only: re-key this connection to another tab, so unit tests can
    /// pin that minted-id namespaces derive from the tab id rather than a
    /// hard-coded `tab0:` literal (#1463 review).
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn set_tab_for_tests(&mut self, tab: TabId) {
        self.tab = tab;
    }

    /// Test-only: abort the feed task owning the client. Harness paths that
    /// swap this connection out for a disconnected stub MUST call this on
    /// the replaced connection, or the orphaned task later injects a
    /// spurious `Closed` sentinel the swap semantics never implied.
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn abort_feed(&self) {
        if let Some(task) = &self.feed_task {
            task.abort();
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
            tab: TabId::MASTER,
            sender: client.clone_sender(),
            speaks_frames: client.speaks_frames(),
            dropped_oversized: client.dropped_oversized_handle(),
            feed_task: None,
        }
    }

    /// Test-only: a connection with a live writer channel. The returned
    /// receiver must be held open for `try_send` to succeed (#1465 AC8).
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn live_for_tests() -> (Self, tokio::sync::mpsc::Receiver<String>) {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(16);
        let sender = crate::protocol::client::CommandSender::from_tx_for_tests(tx);
        (
            Self {
                tab: TabId::MASTER,
                sender,
                speaks_frames: true,
                dropped_oversized: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
                feed_task: None,
            },
            rx,
        )
    }

    /// Test-only: force the ADR-0008 negotiation flag for isolation checks.
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn set_speaks_frames_for_tests(&mut self, speaks: bool) {
        self.speaks_frames = speaks;
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

/// Compose a feed correlation id under a connection namespace — the ONE
/// composition site (#1472 r2), so the namespace encoding cannot drift
/// between hand-rolled `format!` copies.
pub(crate) fn feed_id(ns: &str, suffix: &str) -> String {
    format!("{ns}{suffix}")
}
