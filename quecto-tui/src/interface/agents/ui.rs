use crate::infrastructure::client::{Command, Event, SubagentInfoEvent, SubagentWorkflow};
use crate::interface::agents::feed::{FeedAuthority, FeedSyncState};
use crate::interface::agents::focus::Focus;
use crate::interface::agents::roster::{RosterInfo, TrackedSubagent, subagent_status_is_active};
use crate::interface::components::chat::Chat;
use crate::interface::components::footer::Footer;
use crate::interface::components::list_navigator::ListNavigator;
use crate::interface::components::workflow_bar;
use std::collections::{BTreeMap, VecDeque};
use tokio::sync::mpsc;

impl RosterInfo for SubagentInfoEvent {
    fn status(&self) -> &str {
        &self.status
    }

    fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
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
    }
}

/// Runtime ownership for a direct child feed. Kept separate from feed sync
/// policy so pure synchronization state has no Tokio channel or task handle.
pub(crate) struct FeedRuntime {
    pub(crate) cmd_tx: mpsc::Sender<Command>,
    pub(crate) handle: tokio::task::JoinHandle<()>,
}

/// Direct-feed state as consumed by the legacy `App` slice. The flattened fields
/// preserve existing call sites while construction flows through separated
/// runtime and synchronization parts above.
pub(crate) struct FeedState {
    pub(crate) cmd_tx: mpsc::Sender<Command>,
    pub(crate) handle: tokio::task::JoinHandle<()>,
    pub(crate) epoch: u64,
    pub(crate) rev: u64,
    pub(crate) last_fresh_at: Option<std::time::Instant>,
    pub(crate) supports_sync: bool,
    pub(crate) pending_rev: Option<u64>,
    pub(crate) transcript: crate::interface::agents::ledger::LedgerTranscript,
    pub(crate) authority: FeedAuthority,
}

pub(crate) fn ledger_entry_to_chat_entry(
    entry: crate::interface::agents::ledger::LedgerEntry,
) -> crate::interface::components::chat::ChatEntry {
    use crate::interface::agents::ledger::LedgerEntry;
    use crate::interface::components::chat::ChatEntry;
    match entry {
        LedgerEntry::User { text } => ChatEntry::User { text },
        LedgerEntry::Assistant { text } => ChatEntry::Assistant {
            text,
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
            parsed_args: serde_json::from_str(&args).ok(),
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

/// Concrete sub-agent / multi-session UI state, grouped by owner (#997) and
/// backed by the pure agents roster/feed/focus policy modules (#1222).
pub(crate) struct SubagentUi {
    /// Client-side subagent state for immediate bar updates (#525).
    /// Updated from tool events (spawn/agent_cmd) and server pushes.
    /// Entries track expiry timestamps for auto-removal (#540).
    pub(crate) tracked: BTreeMap<String, TrackedSubagent<SubagentInfoEvent>>,
    /// Animation frame for the subagent spinner, advanced on each spinner tick.
    pub(crate) frame: usize,
    /// The sub-agent the parent is currently blocked on via `agent_cmd await`,
    /// if any. Rendered as a per-row "awaiting" indicator instead of a shared
    /// spinner line.
    pub(crate) awaited_agent_id: Option<String>,
    /// Per-sub-agent session views, keyed by agent id (#800). The master is not
    /// in this map — it is top-level App state and `active_agent_id == None`.
    pub(crate) sessions: BTreeMap<String, SessionView>,
    /// Insertion order of session ids, for bounded retention eviction (#800).
    pub(crate) session_order: Vec<String>,
    /// The agent whose session is shown in the main body. `None` = master.
    pub(crate) active_agent_id: Option<String>,
    /// Left-panel selection cursor over the flattened (master + tree) rows.
    pub(crate) panel_nav: ListNavigator,
    /// Fan-in for events from direct sub-agent connections (#800).
    pub(crate) event_tx: mpsc::Sender<(String, Event)>,
    pub(crate) event_rx: mpsc::Receiver<(String, Event)>,
    /// Per-subagent synced feed state keyed by agent id.
    pub(crate) feeds: BTreeMap<String, FeedState>,
    /// Which pane has keyboard focus: the editor or the side panel (#802).
    pub(crate) focus: Focus,
}

impl SubagentUi {
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
        let (event_tx, event_rx) = mpsc::channel(256);
        Self {
            tracked: BTreeMap::new(),
            frame: 0,
            awaited_agent_id: None,
            sessions: BTreeMap::new(),
            session_order: Vec::new(),
            active_agent_id: None,
            panel_nav: ListNavigator::new(),
            event_tx,
            event_rx,
            feeds: BTreeMap::new(),
            focus: Focus::Input,
        }
    }
}

/// Per-session state for the multi-session UI (#800/#802/#828).
pub(crate) struct SessionView {
    pub(crate) chat: Chat,
    pub(crate) workflow_bar: workflow_bar::WorkflowBarState,
    /// Whether the child is mid-turn — drives a per-session working indicator.
    pub(crate) running: bool,
    /// This session's OWN status footer (context-window / cost / model gauges, #805).
    pub(crate) footer: Footer,
    /// Grandchild completion notes buffered while mid-turn; flushed at idle (#816).
    pub(crate) deferred_subagent_notes: VecDeque<String>,
    /// History cursors, older-page correlation and the partial-vs-complete
    /// backfill latch (#828/#1050/#1061), owned by a pure policy (#1221).
    pub(crate) history: crate::domain::history_paging::HistoryPaging,
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
            workflow_bar: workflow_bar::WorkflowBarState::default(),
            running: false,
            footer,
            deferred_subagent_notes: VecDeque::new(),
            history: crate::domain::history_paging::HistoryPaging::default(),
            observed_run_state: false,
            active_turn_start: 0,
            tools_this_turn: 0,
            open_tool_calls: 0,
        }
    }
}
