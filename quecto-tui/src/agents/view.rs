//! Agents view / concrete UI adapter state (#1222 / #1257 Phase 4).
//!
//! Owns `SubagentUi`, `SessionView`, flattened `FeedState`, and the
//! ledger-to-chat adapter. Runtime connect handles live in `runtime`.

use crate::agents::feed::{FeedAuthority, FeedSyncState};
use crate::agents::focus::Focus;
use crate::agents::roster::{RosterInfo, TrackedSubagent, subagent_status_is_active};
use crate::agents::runtime::FeedRuntime;
use crate::components::chat::Chat;
use crate::components::footer::Footer;
use crate::components::list_navigator::ListNavigator;
use crate::components::workflow_bar;
use crate::protocol::client::{Command, SubagentInfoEvent, SubagentWorkflow};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use tokio::sync::mpsc;

impl RosterInfo for SubagentInfoEvent {
    fn status(&self) -> &str {
        &self.status
    }

    fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }

    fn agent_uuid(&self) -> Option<&str> {
        self.agent_uuid.as_deref()
    }

    fn display_label(&self) -> &str {
        self.display_name
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(self.agent_id.as_str())
    }

    fn merge_sticky_fields(&mut self, previous: &Self) {
        // Preserve last-known workflow + parent_id when an update omits them
        // (get_subagents carries neither, and would otherwise erase the n/n).
        if self.workflow.is_none() {
            self.workflow = previous.workflow.clone();
        }
        if self.parent_id.is_none() {
            self.parent_id = previous.parent_id.clone();
        }
        if self.agent_uuid.is_none() {
            self.agent_uuid = previous.agent_uuid.clone();
        }
        if self.display_name.is_none() {
            self.display_name = previous.display_name.clone();
        }
        if self.socket_path.is_none() {
            self.socket_path = previous.socket_path.clone();
        }
        if self.pid == 0 && previous.pid != 0 {
            self.pid = previous.pid;
        }
        if self.is_compact() {
            self.last_tool = previous.last_tool.clone();
            self.last_error = previous.last_error.clone();
            self.read_only = previous.read_only;
        }
        // Sparse `get_subagents` refreshes omit the environment metadata a
        // live event reported; keep sticky backend/environment only when the
        // compact row did not make environment membership observable. A compact
        // row with an `environmentRef` is authoritative for that ref; enrich it
        // only from a previous rich environment with the SAME ref. A compact row
        // with no ref represents a removal and must not resurrect stale C1/C2
        // metadata from a richer but older event.
        let sparse = self.execution_backend.is_none();
        if sparse {
            let same_environment_ref = self.environment.as_ref().and_then(|current_env| {
                previous.environment.as_ref().filter(|previous_env| {
                    current_env.environment_ref == previous_env.environment_ref
                })
            });
            if !self.is_compact() || same_environment_ref.is_some() {
                self.execution_backend = previous.execution_backend.clone();
            }
            match (&self.environment, &previous.environment) {
                (None, Some(previous_env)) if !self.is_compact() => {
                    self.environment = Some(previous_env.clone());
                }
                (Some(current_env), Some(previous_env))
                    if current_env.environment_ref == previous_env.environment_ref =>
                {
                    self.environment = Some(previous_env.clone());
                }
                _ => {}
            }
        }
    }
}

/// Direct-feed state as consumed by the legacy `App` slice. The flattened fields
/// preserve existing call sites while construction flows through separated
/// runtime and synchronization parts above.
///
/// This flattening is a deliberate staging step: the split into
/// `runtime::FeedRuntime` + `FeedSyncState` is currently enforced only at
/// construction, so new synchronization fields must be added to `FeedSyncState`
/// (not directly here) until the remaining call sites are migrated to
/// `feed.sync.*` under #1222's follow-up.
pub(crate) struct FeedState {
    pub(crate) cmd_tx: mpsc::Sender<Command>,
    pub(crate) handle: tokio::task::JoinHandle<()>,
    pub(crate) inspection_only: bool,
    pub(crate) epoch: u64,
    pub(crate) rev: u64,
    pub(crate) last_fresh_at: Option<std::time::Instant>,
    pub(crate) supports_sync: bool,
    pub(crate) pending_rev: Option<u64>,
    pub(crate) transcript: crate::agents::ledger::LedgerTranscript,
    pub(crate) authority: FeedAuthority,
}

pub(crate) fn ledger_entry_to_chat_entry(
    entry: crate::agents::ledger::LedgerEntry,
) -> crate::components::chat::ChatEntry {
    use crate::agents::ledger::LedgerEntry;
    use crate::components::chat::ChatEntry;
    match entry {
        LedgerEntry::User { text } => ChatEntry::User { text },
        LedgerEntry::Assistant { text, thinking } => ChatEntry::Assistant {
            text,
            thinking,
            streaming: false,
        },
        LedgerEntry::ToolExecution {
            tool_call_id,
            tool_name,
            args,
            result,
            is_error,
        } => ChatEntry::ToolExecution {
            tool_call_id,
            tool_name,
            parsed_args: crate::protocol::agent_ledger_payloads::parse_tool_args(&args),
            args,
            result,
            is_error,
            duration_ms: None,
        },
    }
}

impl FeedState {
    pub(crate) fn from_parts(runtime: FeedRuntime, sync: FeedSyncState) -> Self {
        Self {
            cmd_tx: runtime.cmd_tx,
            handle: runtime.handle,
            inspection_only: runtime.inspection_only,
            epoch: sync.epoch,
            rev: sync.rev,
            last_fresh_at: sync.last_fresh_at,
            supports_sync: sync.supports_sync,
            pending_rev: sync.pending_rev,
            transcript: sync.transcript,
            authority: sync.authority,
        }
    }
}

/// The per-connection roster half of the sub-agent UI (#1463): everything
/// scoped to ONE tab's agent tree — its tracked children, their sessions and
/// feeds, and which of them is focused. Lives on `ConnectionState`.
pub(crate) struct ConnectionRoster {
    /// Client-side subagent state for immediate bar updates (#525).
    /// Updated from tool events (spawn/agent_cmd) and server pushes.
    /// Entries track expiry timestamps for auto-removal (#540).
    pub(crate) tracked: BTreeMap<String, TrackedSubagent<SubagentInfoEvent>>,
    /// Durable UUIDs for terminal rows already displayed for their grace period.
    /// Suppresses repeated historical terminal snapshots while allowing a
    /// nonterminal restart with the same UUID to reappear.
    pub(crate) expired_terminal_uuids: BTreeSet<String>,
    /// Animation frame for the subagent spinner, advanced on each spinner tick.
    pub(crate) frame: usize,
    /// Per-sub-agent session views, keyed by agent id (#800). The master is
    /// not in this map — it lives on the connection and
    /// `active_agent_id == None` selects it.
    pub(crate) sessions: BTreeMap<String, SessionView>,
    /// Insertion order of session ids, for bounded retention eviction (#800).
    pub(crate) session_order: Vec<String>,
    /// The agent whose session is shown in the main body. `None` = master.
    pub(crate) active_agent_id: Option<String>,
    /// The shared environment whose details render in the main-pane chrome
    /// after its panel row was selected (#1369 slice 4). Stores the grouping
    /// key (`SubagentEnvironmentInfo::group_key`, review #1392), not the
    /// painted ref. `None` = agent chrome.
    pub(crate) selected_environment: Option<String>,
    /// Per-subagent synced feed state keyed by agent id.
    pub(crate) feeds: BTreeMap<String, FeedState>,
}

impl ConnectionRoster {
    /// How many tracked child agents are currently in an active status.
    pub(crate) fn tracked_active_count(&self) -> usize {
        self.tracked
            .values()
            .filter(|t| subagent_status_is_active(&t.info.status))
            .count()
    }

    /// The workflow snapshot tracked for `id`, if any.
    pub(crate) fn tracked_workflow(&self, id: &str) -> Option<&SubagentWorkflow> {
        self.tracked.get(id).and_then(|t| t.info.workflow.as_ref())
    }

    pub(crate) fn new() -> Self {
        Self {
            tracked: BTreeMap::new(),
            expired_terminal_uuids: BTreeSet::new(),
            frame: 0,
            sessions: BTreeMap::new(),
            session_order: Vec::new(),
            active_agent_id: None,
            selected_environment: None,
            feeds: BTreeMap::new(),
        }
    }
}

/// The global UI half of the sub-agent state (#997/#1463): panel focus and
/// cursor are app chrome shared across tabs, and the event channel is the
/// shared fan-in transport for every tab's feeds.
pub(crate) struct SubagentUi {
    /// Left-panel selection cursor over the flattened (master + tree) rows.
    pub(crate) panel_nav: ListNavigator,
    /// Durable identity for the focused panel cursor. The row index is only a
    /// viewport coordinate; live roster updates can reorder rows, so focused
    /// navigation preserves/commits by this key when possible.
    pub(crate) panel_nav_key: Option<String>,
    /// Shared fan-in for the tab's master connection AND its direct/routed
    /// sub-agent feeds (#800/#1442/#1462), keyed by
    /// [`crate::shell::connection::SourcedEvent`] so the event loop drains ONE
    /// channel regardless of connection count.
    pub(crate) event_tx: mpsc::Sender<crate::shell::connection::SourcedEvent>,
    pub(crate) event_rx: mpsc::Receiver<crate::shell::connection::SourcedEvent>,
    /// Which pane has keyboard focus: the editor or the side panel (#802).
    pub(crate) focus: Focus,
}

impl SubagentUi {
    pub(crate) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        Self {
            panel_nav: ListNavigator::new(),
            panel_nav_key: None,
            event_tx,
            event_rx,
            focus: Focus::Input,
        }
    }
}

/// Max entries retained in `SessionView::live_inflight` (#1259 review).
/// Older entries are dropped with a truncation marker so a long/noisy
/// unfocused turn cannot grow a second unbounded transcript.
pub(crate) const LIVE_INFLIGHT_ENTRY_CAP: usize = 256;

/// Per-session state for the multi-session UI (#800/#802/#828).
pub(crate) struct SessionView {
    pub(crate) chat: Chat,
    /// Retained in-flight turn stream for a synced/warm child (#1259). Survives
    /// focus changes and mid-turn ledger re-projection; cleared on epoch/resync
    /// or when an idle turn's ledger advances past the live tail. Not shown for
    /// unfocused children until focus/reproject merges it onto `chat`.
    pub(crate) live_inflight: Chat,
    pub(crate) workflow_bar: workflow_bar::WorkflowBarState,
    /// Whether the child is mid-turn — drives a per-session working indicator.
    pub(crate) running: bool,
    /// This session's OWN status footer (context-window / cost / model gauges, #805).
    pub(crate) footer: Footer,
    /// Grandchild completion notes buffered while mid-turn; flushed at idle (#816).
    pub(crate) deferred_subagent_notes: VecDeque<String>,
    /// History cursors, older-page correlation and the partial-vs-complete
    /// backfill latch (#828/#1050/#1061), owned by a pure policy (#1221).
    pub(crate) history: crate::conversation::history_paging::HistoryPaging,
    /// Until this session's own stream reports run-state, `active_subagent_running`
    /// trusts the tracked status not `running` (#834).
    pub(crate) observed_run_state: bool,
    /// Chat entry index at which this child's active turn began.
    pub(crate) active_turn_start: usize,
    pub(crate) tools_this_turn: usize,
    /// Child tool starts not yet ended; forces recovery on a dropped end (review 3).
    pub(crate) open_tool_calls: usize,
}

impl SessionView {
    pub(crate) fn new(git_branch: Option<String>) -> Self {
        let mut footer = Footer::new();
        footer.set_git_branch(git_branch);
        Self::with_footer(footer)
    }

    /// Build a session around a pre-configured footer — used for both sub-agent
    /// and master (#828) sessions, so all are constructed identically.
    pub(crate) fn with_footer(footer: Footer) -> Self {
        Self {
            chat: Chat::new(),
            live_inflight: Chat::new(),
            workflow_bar: workflow_bar::WorkflowBarState::default(),
            running: false,
            footer,
            deferred_subagent_notes: VecDeque::new(),
            history: crate::conversation::history_paging::HistoryPaging::default(),
            observed_run_state: false,
            active_turn_start: 0,
            tools_this_turn: 0,
            open_tool_calls: 0,
        }
    }

    /// Rebuild `chat` from committed ledger entries (#1259).
    ///
    /// - `clear_live`: drop the retained in-flight buffer (ledger supersedes it).
    /// - `attach_live`: append the retained buffer onto `chat` (focused mid-turn).
    ///
    /// Unfocused re-projection uses `attach_live = false` so chat stays
    /// ledger-authoritative while the buffer survives until focus or supersession.
    pub(crate) fn project_ledger_with_live(
        &mut self,
        ledger: impl IntoIterator<Item = crate::agents::ledger::LedgerEntry>,
        attach_live: bool,
        clear_live: bool,
    ) {
        if clear_live {
            self.live_inflight.clear();
        }
        let preserved_scroll_offset = self.chat.scroll_offset();
        self.chat.clear();
        if preserved_scroll_offset > 0 {
            self.chat.scroll_up(preserved_scroll_offset);
        }
        for entry in ledger {
            self.chat
                .add_entry(crate::agents::view::ledger_entry_to_chat_entry(entry));
        }
        // Projection rebuilds are authoritative snapshots. Discard trim deltas
        // accumulated while rebuilding the committed prefix so callers only
        // reconcile retention that happens after the live boundary is reset.
        let _ = self.chat.take_retention_front_delta();
        let committed_entry_count = self.chat.entry_count();
        if attach_live {
            self.active_turn_start = committed_entry_count;
            // Skip live tool cards already present in the committed ledger so a
            // mid-turn tool checkpoint does not double-render (#1259 review).
            let ledger_tool_ids: std::collections::HashSet<String> = self
                .chat
                .entries()
                .iter()
                .filter_map(|e| match e {
                    crate::components::chat::ChatEntry::ToolExecution { tool_call_id, .. } => {
                        Some(tool_call_id.clone())
                    }
                    _ => None,
                })
                .collect();
            let live_entries: Vec<_> = self.live_inflight.entries().to_vec();
            for entry in live_entries {
                if let crate::components::chat::ChatEntry::ToolExecution { tool_call_id, .. } =
                    &entry
                {
                    if ledger_tool_ids.contains(tool_call_id) {
                        continue;
                    }
                }
                self.chat.add_entry(entry);
            }
        }
    }

    /// Drop oldest live-inflight entries past [`LIVE_INFLIGHT_ENTRY_CAP`],
    /// leaving a single truncation status so overflow is visible (#1259).
    pub(crate) fn reconcile_chat_retention_trim(&mut self) {
        let (trimmed, inserted) = self.chat.take_retention_front_delta();
        if trimmed > 0 || inserted > 0 {
            self.active_turn_start = self
                .active_turn_start
                .saturating_sub(trimmed)
                .saturating_add(inserted);
        }
    }

    pub(crate) fn cap_live_inflight(&mut self) {
        let n = self.live_inflight.entry_count();
        if n <= LIVE_INFLIGHT_ENTRY_CAP {
            return;
        }
        // Keep the newest (cap - 1) entries and prepend a truncation marker.
        let keep = LIVE_INFLIGHT_ENTRY_CAP.saturating_sub(1);
        let start = n.saturating_sub(keep);
        let kept: Vec<_> = self.live_inflight.entries()[start..].to_vec();
        self.live_inflight.clear();
        self.live_inflight
            .add_entry(crate::components::chat::ChatEntry::Status {
                text: "… earlier live output truncated …".into(),
            });
        for entry in kept {
            self.live_inflight.add_entry(entry);
        }
    }
}
