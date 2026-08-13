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
    pub(crate) agent_connected: bool,
    /// Pin: once the left panel has shown for a connected agent it must not
    /// vanish when the agent dies (#1047) — the user keeps the session /
    /// sub-agent context to diagnose the failure. Stays `true` on disconnect.
    pub(crate) agent_ever_connected: bool,
    /// Exit-diagnosis watch for the TUI-owned agent child (#1047), published
    /// by [`crate::shell::child_watch`]. `None` for external sockets.
    pub(crate) child_exit_watch: Option<crate::shell::child_watch::ChildWatch>,
    /// OS pid of the TUI-owned agent child when known (registry sidecar, AC4).
    pub(crate) child_pid: Option<u32>,
    /// Live UDS path for this tab's master agent (registry/reattach, AC4/AC6).
    pub(crate) socket_path: Option<std::path::PathBuf>,
    /// Durable session key for this tab's master agent (manifest, AC4/AC5/AC6).
    pub(crate) session_key: Option<String>,
    /// Session key to resume once this tab becomes connected (workspace restore).
    pub(crate) pending_session_resume: Option<String>,
    /// True while a background spawn/reattach for this tab is in flight (AC2).
    pub(crate) pending_attach: bool,
    /// Generation stamped when the current attach/spawn was kicked; outcomes
    /// with a mismatched generation are rejected so close→reopen cannot attach
    /// the wrong agent into a recycled TabId (#1465 F2).
    pub(crate) attach_generation: u64,
    /// Draft text composed while this tab was focused (swapped on tab switch).
    pub(crate) editor_draft: String,
    /// Prompts typed while attach was still pending; flushed after connect.
    pub(crate) queued_prompts: Vec<String>,
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
    /// Bundle a freshly spawned transport with the connected-tab defaults.
    pub(crate) fn new(
        transport: crate::shell::connection::Connection,
        master_session: SessionView,
    ) -> Self {
        Self {
            transport,
            name: None,
            master_session,
            agent_state: AgentRunState::new(),
            spinner: None,
            connected_agent_id: None,
            agent_connected: true,
            agent_ever_connected: true,
            child_exit_watch: None,
            child_pid: None,
            socket_path: None,
            session_key: None,
            pending_session_resume: None,
            pending_attach: false,
            attach_generation: 0,
            editor_draft: String::new(),
            queued_prompts: Vec::new(),
            surfaced_oversized_drops: 0,
            disconnect_diag_pending: false,
            disconnect_refusal_notified: false,
            started_at: tokio::time::Instant::now(),
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

    /// The correlation-id namespace prefix for this connection's tab
    /// (#1463): `tab{N}:`. Every id the tab mints carries it, so broadcast
    /// responses can never match another tab's pending latches.
    pub(crate) fn id_namespace(&self) -> String {
        // Kept as an owned String for call-site compatibility; derived from
        // the tab id so it can never drift from the transport (#1463).
        format!("tab{}:", self.transport.tab().0)
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
            .unwrap_or("Master")
    }

    /// The label to render for this tab's pinned master panel row. N=1 keeps
    /// the legacy row text byte-identical; named tabs paint the tab name.
    pub(crate) fn master_panel_label(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("Master Agent")
    }
}

impl App {
    /// Resolve which tab `active_conn*` should address: routing override
    /// (inbound event owner) if set, otherwise the focused `active_tab`.
    fn effective_tab(&self) -> crate::shell::connection::TabId {
        self.routing_tab_override.unwrap_or(self.active_tab)
    }

    /// The focused (or routing-override) tab's connection state.
    pub(crate) fn active_conn(&self) -> &ConnectionState {
        let tab = self.effective_tab();
        self.tabs
            .get(&tab)
            .unwrap_or_else(|| panic!("missing connection state for effective tab {tab:?}"))
    }

    /// Mutable counterpart to [`Self::active_conn`].
    pub(crate) fn active_conn_mut(&mut self) -> &mut ConnectionState {
        let tab = self.effective_tab();
        self.conn_mut(tab)
            .unwrap_or_else(|| panic!("missing connection state for effective tab {tab:?}"))
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

    /// Immutable lookup for a specific tab (None if unknown).
    pub(crate) fn conn_for(
        &self,
        tab: crate::shell::connection::TabId,
    ) -> Option<&ConnectionState> {
        self.tabs.get(&tab)
    }

    /// Mutable lookup for a specific tab (None if unknown).
    pub(crate) fn conn_mut(
        &mut self,
        tab: crate::shell::connection::TabId,
    ) -> Option<&mut ConnectionState> {
        self.tabs.get_mut(&tab)
    }

    /// Run `f` with `active_conn*` temporarily addressing `tab` so existing
    /// handlers can mutate the event owner without changing focus (#1465).
    /// Unknown tabs are a no-op (AC7: no ghost state, no active fallthrough).
    pub(crate) fn with_routing_tab<R>(
        &mut self,
        tab: crate::shell::connection::TabId,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        self.conn_for(tab)?;
        let prev = self.routing_tab_override;
        self.routing_tab_override = Some(tab);
        let out = f(self);
        self.routing_tab_override = prev;
        Some(out)
    }

    /// Close global overlay surfaces when switching the active tab/session.
    /// N=1 session switches use the same seam; this preserves compose-frame
    /// idempotence by doing the state transition outside render composition.
    pub(crate) fn close_tab_switch_overlays(&mut self) {
        let conn = self.active_conn_mut();
        conn.sessions.resume_selector = None;
        conn.rewind.selector = None;
        self.autocomplete.dismiss();
        self.workspace.files_autocomplete.dismiss();
        self.tool_policy_modal = None;
        self.tool_policy_modal_pending_catalogue_id = None;
        self.inference.model_selector = None;
        self.inference.effort_selector = None;
        // Global model-selector open latch must not fire on the newly focused
        // tab after a switch (#1465 F10).
        self.inference.model_registry.open_pending = false;
    }
}
