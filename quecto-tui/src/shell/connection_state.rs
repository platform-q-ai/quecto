//! Per-tab connection state (#1463, epic #1467).
//!
//! Phase 2 of the multi-session TUI bundles everything scoped to ONE master
//! connection — its transport handle plus the agent-lifecycle state that
//! phase 1 left on `App` — into this struct, reached through
//! [`App::active_conn`] / [`App::active_conn_mut`]. At N=1 the app owns
//! exactly one; the accessors are the dispatch seam where tab selection
//! lands with N>1, mirroring the proven `active_session_mut()` seam.

use super::*;

/// Everything owned by one tab's master connection. Move order follows the
/// issue's blast-radius clusters; fields arrive cluster by cluster.
pub(crate) struct ConnectionState {
    /// The tab's transport: the master connection behind its feed task
    /// (#1462). The feed task owns the [`Client`]; this is the command/state
    /// handle.
    pub(crate) transport: crate::shell::connection::Connection,
    /// Agent run state machine (abort-aware, #502).
    pub(crate) agent_state: AgentRunState,
    /// Working spinner for the tab's own agent turn; `None` when idle.
    pub(crate) spinner: Option<Spinner>,
    /// Connected agent's own id (get_state sessionKey), vs descendants' (#997).
    pub(crate) connected_agent_id: Option<String>,
    pub(crate) agent_connected: bool,
    /// Pin: once the left panel has shown for a connected agent it must not
    /// vanish when the agent dies (#1047) — the user keeps the session /
    /// sub-agent context to diagnose the failure. Stays `true` on disconnect.
    pub(crate) agent_ever_connected: bool,
    /// Exit-diagnosis watch for the TUI-owned agent child (#1047), published
    /// by [`crate::shell::child_watch`]. `None` for external sockets.
    pub(crate) child_exit_watch: Option<crate::shell::child_watch::ChildWatch>,
    /// Oversized-event drops already surfaced as a notification, so each is
    /// reported exactly once (#1047).
    pub(crate) surfaced_oversized_drops: u64,
    /// Whether a stream-closed disconnect diagnosis is resolving off-loop
    /// (#1462 scope 3) for THIS tab: set by `begin_agent_stream_closed` when
    /// it spawns the bounded #1047 waits, cleared by
    /// `finish_agent_stream_closed` for the matching tab only (#1463). The
    /// harness keys its diagnosis pumping off this latch.
    pub(crate) disconnect_diag_pending: bool,
    /// One "commands are not being sent" notice per disconnect episode
    /// (#1470 r4): reset when a disconnect begins, set on first refusal.
    pub(crate) disconnect_refusal_notified: bool,
    /// When this tab's session started — drives the Master row's uptime
    /// timer (#820).
    pub(crate) started_at: tokio::time::Instant,
}

impl ConnectionState {
    /// Bundle a freshly spawned transport with the connected-tab defaults.
    pub(crate) fn new(transport: crate::shell::connection::Connection) -> Self {
        Self {
            transport,
            agent_state: AgentRunState::new(),
            spinner: None,
            connected_agent_id: None,
            agent_connected: true,
            agent_ever_connected: true,
            child_exit_watch: None,
            surfaced_oversized_drops: 0,
            disconnect_diag_pending: false,
            disconnect_refusal_notified: false,
            started_at: tokio::time::Instant::now(),
        }
    }
}

impl App {
    /// The active tab's connection state. N=1: always the master tab.
    ///
    /// This pair is the tab-dispatch seam (#1463): with N>1 tabs the lookup
    /// by active tab id lands here, exactly like `active_session_mut()`
    /// dispatches over sessions. Call sites may also reach the state through
    /// the `conn` field directly where split borrows require a plain place
    /// expression.
    pub(crate) fn active_conn(&self) -> &ConnectionState {
        &self.conn
    }

    /// Mutable counterpart to [`Self::active_conn`].
    pub(crate) fn active_conn_mut(&mut self) -> &mut ConnectionState {
        &mut self.conn
    }
}
