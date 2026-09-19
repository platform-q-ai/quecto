//! Connection state (#1463; single connection since #2044).
//!
//! Everything scoped to the TUI's ONE master connection — its transport
//! handle plus the agent-lifecycle state — lives in this struct, owned by
//! `App` and reached through [`App::active_conn`] / [`App::active_conn_mut`].

use super::*;

/// The correlation-id prefix every minted request id carries (#1463). One
/// constant for the one connection; ownership of an answer is decided by
/// exact pending-id equality, never by this prefix.
pub(crate) const ID_NAMESPACE: &str = "tab0:";

/// Everything owned by the master connection. Move order follows the
/// issue's blast-radius clusters; fields arrive cluster by cluster.
pub(crate) struct ConnectionState {
    /// The tab's transport: the master connection behind its feed task
    /// (#1462). The feed task owns the [`Client`]; this is the command/state
    /// handle.
    pub(crate) transport: crate::shell::connection::Connection,
    /// Optional human label for this tab. N=1 leaves it unset so rendered
    /// frames stay byte-identical; N>1 tab creation can set it and the render
    /// path will paint it wherever the active tab's master is named (#1464).
    pub(crate) name: Option<String>,
    /// The tab's master agent session, modeled as just another
    /// [`SessionView`] (#828) so render/input share ONE active-session path
    /// with sub-agents (`active_agent_id == None` selects this). Only
    /// `spinner`/`agent_state` stay master-local; sub-agents derive
    /// `running` from forwarded events.
    pub(crate) master_session: SessionView,
    /// Agent run state machine (abort-aware, #502).
    pub(crate) agent_state: AgentRunState,
    /// Working spinner for the tab's own agent turn; `None` when idle.
    pub(crate) spinner: Option<Spinner>,
    /// Connected agent's own id (get_state sessionKey), vs descendants' (#997).
    pub(crate) connected_agent_id: Option<String>,
    /// Local monotonic receipt time of the authoritative master snapshot.
    pub(crate) admission_observed_at: tokio::time::Instant,
    /// Bounded by the tracked roster; snapshots retain their original clock anchor.
    pub(crate) admission_children: std::collections::BTreeMap<
        String,
        (
            crate::protocol::admission_payloads::AdmissionView,
            tokio::time::Instant,
        ),
    >,
    /// Children whose latest wait clear came from an unversioned terminal event.
    pub(crate) admission_unversioned_clears: std::collections::BTreeSet<String>,
    /// Last admission view; absent without an authority or after disconnect.
    pub(crate) admission_view: Option<crate::protocol::admission_payloads::AdmissionView>,
    /// The spinner message this module last wrote, so a tool message set by
    /// someone else is never clobbered when the wait ends.
    pub(crate) admission_spinner_message: Option<String>,
    pub(crate) agent_connected: bool,
    /// Pin: once the left panel has shown for a connected agent it must not
    /// vanish when the agent dies (#1047) — the user keeps the session /
    /// sub-agent context to diagnose the failure. Stays `true` on disconnect.
    pub(crate) agent_ever_connected: bool,
    /// Exit-diagnosis watch for the TUI-owned agent child (#1047), published
    /// by [`crate::shell::child_watch`]. `None` for external sockets.
    pub(crate) child_exit_watch: Option<crate::shell::child_watch::ChildWatch>,
    /// Durable session key of this connection's master agent.
    pub(crate) session_key: Option<String>,
    /// Request id of this tab's in-flight `resume_session`, so only its own
    /// answer clears the resume latches; foreign answers (another client
    /// resuming the shared agent) still refresh the view (#1726).
    pub(crate) pending_session_resume_id: Option<String>,
    /// Whether the resume in flight carried an explicit action (#2011): only
    /// such a request can be what an "unknown resume action" parse error is about.
    pub(crate) pending_session_resume_acts: bool,
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
    /// Origin of the Coordinator row's cumulative active-processing clock
    /// (#820/#838, #1726): `now - started_at` is the session's total active
    /// time while running. Restarting after an idle gap shifts this origin
    /// forward by the gap, so idle time never counts.
    pub(crate) started_at: tokio::time::Instant,
    /// When the clock was frozen. `None` while actively running;
    /// `Some(started_at)` at a session boundary so idle frames show `0:00`
    /// instead of a wall-clock session uptime.
    pub(crate) stopped_at: Option<tokio::time::Instant>,
    /// In-flight #1060 fetch-on-miss recoveries keyed by minted request id.
    pub(crate) pending_message_recovery: HashMap<String, PendingMessageRecovery>,
    /// Recovery batches (client-local id → turn chat range) guarding late
    /// overwrites.
    pub(crate) message_recovery_batches: HashMap<String, MessageRecoveryBatch>,
    pub(crate) pending_stub_recall: HashMap<String, app_paged_history::StubRecall>,
    pub(crate) failed_stub_recalls: HashSet<(Option<String>, String)>,
    /// Exact correlation id for this tab's in-flight resume transcript fetch
    /// (#1237). `get_messages` responses are broadcast; fixed literals would
    /// clobber peers.
    pub(crate) pending_resume_messages_id: Option<String>,
    /// Exact correlation id for this tab's post-rewind transcript refresh (#1237).
    pub(crate) pending_rewind_refresh_id: Option<String>,
    /// Exact correlation id for this tab's solicited attach backfill (#1237).
    /// Id-less busy-connect snapshots must not clear this pending.
    pub(crate) pending_attach_backfill_id: Option<String>,
    /// Local sequence suffix for minted solicited `get_messages` ids (#1237).
    pub(crate) solicited_get_messages_seq: u64,
    /// Rewind flow state (#997) for this tab's conversation.
    pub(crate) rewind: RewindFlow,
    /// Session pick/resume flow state for this tab's agent.
    pub(crate) sessions: SessionsFlow,
    /// Workflow flow state for this tab's agent.
    pub(crate) workflow: WorkflowFlow,
    /// The model/effort the tab's agent currently runs with (#1463);
    /// selector overlays stay global on `App`.
    pub(crate) inference: app_inference::ConnInference,
    /// This tab's agent tree: tracked children, their sessions and feeds
    /// (#1463 cluster 6); the panel focus/cursor half stays global.
    pub(crate) roster: crate::agents::view::ConnectionRoster,
}

impl ConnectionState {
    /// Resume the cumulative Coordinator active-runtime clock: the frozen
    /// duration carries over and the idle gap is excluded. A start while
    /// already running is a no-op.
    pub(crate) fn start_coordinator_clock(&mut self, now: tokio::time::Instant) {
        if let Some(stopped_at) = self.stopped_at {
            let elapsed = stopped_at.saturating_duration_since(self.started_at);
            self.started_at = now - elapsed;
            self.stopped_at = None;
        }
    }

    /// Freeze the cumulative clock. A stop while already frozen (a late
    /// AgentEnd after an abort, a disconnect after the end) is a no-op, so
    /// it cannot count an idle gap.
    pub(crate) fn stop_coordinator_clock(&mut self, now: tokio::time::Instant) {
        if self.stopped_at.is_none() {
            self.stopped_at = Some(now);
        }
    }

    /// Restart the clock at `0:00`, frozen: a session boundary (new session,
    /// resume into a different session, fresh attach).
    pub(crate) fn reset_coordinator_clock(&mut self, now: tokio::time::Instant) {
        self.started_at = now;
        self.stopped_at = Some(now);
    }

    /// Bundle a freshly spawned transport with the connected-tab defaults.
    pub(crate) fn new(
        transport: crate::shell::connection::Connection,
        master_session: SessionView,
    ) -> Self {
        let started_at = tokio::time::Instant::now();
        Self {
            transport,
            name: None,
            master_session,
            agent_state: AgentRunState::new(),
            spinner: None,
            connected_agent_id: None,
            admission_observed_at: tokio::time::Instant::now(),
            admission_children: Default::default(),
            admission_unversioned_clears: Default::default(),
            admission_view: None,
            admission_spinner_message: None,
            agent_connected: true,
            agent_ever_connected: true,
            child_exit_watch: None,
            session_key: None,
            pending_session_resume_id: None,
            pending_session_resume_acts: false,
            surfaced_oversized_drops: 0,
            disconnect_diag_pending: false,
            disconnect_refusal_notified: false,
            started_at,
            stopped_at: Some(started_at),
            pending_message_recovery: HashMap::new(),
            message_recovery_batches: HashMap::new(),
            pending_stub_recall: HashMap::new(),
            failed_stub_recalls: HashSet::new(),
            pending_resume_messages_id: None,
            pending_rewind_refresh_id: None,
            pending_attach_backfill_id: None,
            solicited_get_messages_seq: 0,
            rewind: RewindFlow::default(),
            sessions: SessionsFlow::default(),
            workflow: WorkflowFlow::default(),
            inference: app_inference::ConnInference::default(),
            roster: crate::agents::view::ConnectionRoster::new(),
        }
    }

    /// The correlation-id namespace prefix every minted id carries (#1463).
    /// A constant since the TUI became a single-connection client (#2044):
    /// the wire format of ids is unchanged.
    pub(crate) fn id_namespace(&self) -> String {
        // Kept as an owned String for call-site compatibility.
        ID_NAMESPACE.to_string()
    }

    /// Mint `suffix` under this connection's namespace (#1463).
    pub(crate) fn namespaced_id(&self, suffix: &str) -> String {
        format!("{}{suffix}", self.id_namespace())
    }

    /// The label to render for this tab's main-pane title.
    pub(crate) fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("Coordinator")
    }

    /// The label to render for this tab's pinned coordinator panel row. N=1 uses
    /// the Coordinator fallback; named tabs paint the tab name.
    pub(crate) fn master_panel_label(&self) -> &str {
        self.display_name()
    }
}

impl App {
    /// The TUI's one connection state.
    pub(crate) fn active_conn(&self) -> &ConnectionState {
        &self.conn
    }

    /// Mutable counterpart to [`Self::active_conn`].
    pub(crate) fn active_conn_mut(&mut self) -> &mut ConnectionState {
        &mut self.conn
    }

    /// Short alias for dense call sites (line-budget / rustfmt).
    #[inline]
    pub(crate) fn ac(&self) -> &ConnectionState {
        self.active_conn()
    }

    /// Short alias for dense call sites (line-budget / rustfmt).
    #[inline]
    pub(crate) fn ac_mut(&mut self) -> &mut ConnectionState {
        self.active_conn_mut()
    }

    /// Close global overlay surfaces when switching the active session. This
    /// preserves compose-frame idempotence by doing the state transition
    /// outside render composition.
    pub(crate) fn close_session_switch_overlays(&mut self) {
        let conn = self.active_conn_mut();
        conn.sessions.close_picker();
        conn.sessions.resume_decision = None;
        conn.rewind.selector = None;
        self.autocomplete.dismiss();
        self.workspace.files_autocomplete.dismiss();
        self.tool_policy_modal = None;
        self.tool_policy_modal_pending_catalogue_id = None;
        self.inference.model_selector = None;
        self.inference.effort_selector = None;
        // Global model-selector open latch must not fire on the newly focused
        // session after a switch (#1465 F10).
        self.inference.model_registry.open_pending = false;
    }
}
