use super::protocol::{AgentEvent, SessionState};
use super::uds::DispatchCtx;
use super::uds_session::{
    HISTORY_PAGE_SIZE, compute_session_stats_with_usage, message_to_json_for_history_page,
    messages_page_json,
};
use crate::domain::message::{Message, Role, ThinkingBlock};
use crate::domain::session::ContextSpillStore;
use std::collections::VecDeque;
use std::sync::Arc;
pub(super) fn is_injected_system_prompt(message: &Message, prompt: &str) -> bool {
    !prompt.is_empty()
        && message.role == Role::System
        && !message.is_manifest
        && message.content == prompt
}
pub(super) fn user_visible_messages(messages: &[Message], system_prompt: &str) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| !is_injected_system_prompt(m, system_prompt))
        .cloned()
        .collect()
}
pub(crate) type StateSnapshot = std::sync::Arc<tokio::sync::RwLock<SessionState>>;
/// Bounded id-addressable ledger budgets. Eviction is oldest-first and triggers
/// on either content bytes or entry count so long-running sessions and floods of
/// tiny messages cannot grow memory unbounded (#1060 review r4).
const LEDGER_MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const LEDGER_MAX_ENTRIES: usize = 8192;
/// Fixed per-entry overhead so a zero/tiny-content message still consumes
/// budget: it covers the id `String` stored twice (map
/// key + order deque, ~2×UUID) plus the owned Message/ToolCall struct footprint.
const LEDGER_ENTRY_OVERHEAD: usize = 256;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LedgerAdvance {
    pub epoch: u64,
    pub rev: u64,
    pub changed: bool,
}
impl LedgerAdvance {
    fn unchanged(epoch: u64, rev: u64) -> Self {
        Self {
            epoch,
            rev,
            changed: false,
        }
    }
}
/// Approximate owned in-memory size of a ledger entry for byte-budgeting.
fn message_bytes(m: &Message) -> usize {
    LEDGER_ENTRY_OVERHEAD
        + m.content.len()
        + m.tool_calls
            .iter()
            .map(|tc| tc.arguments.len() + tc.name.len() + tc.id.len())
            .sum::<usize>()
        + m.tool_call_id.as_ref().map_or(0, |s| s.len())
        + m.tool_name.as_ref().map_or(0, |s| s.len())
        + m.thinking_blocks
            .iter()
            .map(thinking_block_bytes)
            .sum::<usize>()
}

fn thinking_block_bytes(tb: &ThinkingBlock) -> usize {
    match tb {
        ThinkingBlock::Normal {
            thinking,
            signature,
        } => thinking.len() + signature.len(),
        ThinkingBlock::Redacted { data } => data.len(),
    }
}
/// Busy-path conversation snapshot: pruned live messages plus a bounded
/// id→message ledger for resolving recent end-of-turn refs after pruning.
#[derive(Default)]
pub(crate) struct ConversationSnapshotData {
    /// Live conversation as last published (may be pruned/collapsed).
    pub messages: Vec<Message>,
    /// id → full message, bounded by byte + entry caps, authoritative for
    /// `get_message`.
    ledger: std::collections::HashMap<String, Message>,
    /// Insertion order (front = oldest) driving eviction.
    ledger_order: std::collections::VecDeque<String>,
    /// Running byte total of `ledger` (content + per-entry overhead).
    ledger_bytes: usize,
    /// Spill store used as a best-effort backstop when a live message only has
    /// a collapsed recall stub and its full ledger copy has already been
    /// evicted. This is captured in the shared snapshot so the busy reader path
    /// can resolve refs while the main dispatch loop is blocked (#1093).
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    /// Session key paired with [`spill_store`].
    spill_session_key: String,
    /// Monotonic identity/version for the published history. Deferred spill
    /// reads capture this value and may only complete while it is unchanged,
    /// preventing an old session/rewind lookup from crossing a lifecycle
    /// replacement while the snapshot lock is released for I/O.
    generation: u64,
    pub epoch: u64,
    pub rev: u64,
    frontier: VecDeque<(u64, String)>,
}
impl ConversationSnapshotData {
    /// Fold one message into the ledger under the byte budget. `overwrite`
    /// replaces an existing (possibly collapsed) entry with a full copy; without
    /// it, an earlier full copy is kept (publish must not clobber it).
    fn remember(&mut self, m: &Message, overwrite: bool) -> bool {
        let id = m.id().to_string();
        let already = self.ledger.contains_key(&id);
        if already && !overwrite {
            return false;
        }
        let new_sz = message_bytes(m);
        if already {
            let old_sz = self.ledger.get(&id).map(message_bytes).unwrap_or(0);
            self.ledger_bytes = self.ledger_bytes.saturating_sub(old_sz);
            self.ledger.insert(id, m.clone()); // keeps its original age in `ledger_order`
        } else {
            self.ledger.insert(id.clone(), m.clone());
            self.ledger_order.push_back(id);
        }
        self.ledger_bytes += new_sz;
        // Evict oldest-first while EITHER cap is exceeded — the byte budget
        // bounds large-payload content, the entry cap bounds a flood of
        // tiny/empty messages whose per-entry cost the byte total under-counts.
        while self.ledger_bytes > LEDGER_MAX_BYTES || self.ledger.len() > LEDGER_MAX_ENTRIES {
            let Some(old_id) = self.ledger_order.pop_front() else {
                break;
            };
            if let Some(old) = self.ledger.remove(&old_id) {
                self.ledger_bytes = self.ledger_bytes.saturating_sub(message_bytes(&old));
                if let Some(pos) = self.frontier.iter().position(|(_, fid)| fid == &old_id) {
                    self.frontier.remove(pos);
                }
            }
        }
        !already
    }
    fn advance_for_new_ids(&mut self, ids: Vec<String>) -> LedgerAdvance {
        if ids.is_empty() {
            return LedgerAdvance::unchanged(self.epoch, self.rev);
        }
        for id in ids {
            self.rev = self.rev.wrapping_add(1);
            self.frontier.push_back((self.rev, id));
        }
        LedgerAdvance {
            epoch: self.epoch,
            rev: self.rev,
            changed: true,
        }
    }
    /// Replace the live view and fold its messages into the ledger WITHOUT
    /// overwriting existing entries — a full copy recorded earlier must survive
    /// a later in-place collapse of the live message.
    pub fn publish(&mut self, messages: &[Message]) -> LedgerAdvance {
        let mut new_ids = Vec::new();
        for m in messages {
            if self.remember(m, false) {
                new_ids.push(m.id().to_string());
            }
        }
        self.messages = messages.to_vec();
        self.advance_for_new_ids(new_ids)
    }
    pub fn record_full(&mut self, messages: &[Message]) -> LedgerAdvance {
        let mut new_ids = Vec::new();
        for m in messages {
            if self.remember(m, true) {
                new_ids.push(m.id().to_string());
            }
        }
        self.advance_for_new_ids(new_ids)
    }
    /// Attach a spill store for best-effort recall of live collapsed messages
    /// whose full ledger copy is no longer available.
    pub fn set_spill_store(
        &mut self,
        spill_store: Option<Arc<dyn ContextSpillStore>>,
        session_key: String,
    ) {
        // A changed namespace invalidates any deferred read prepared against
        // the previous spill store/session identity. `Arc::ptr_eq` avoids
        // advancing the generation during ordinary per-turn refreshes, which
        // re-publish the same store handle.
        let store_changed = match (&self.spill_store, &spill_store) {
            (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
            (None, None) => false,
            _ => true,
        };
        if self.spill_session_key != session_key || store_changed {
            self.generation = self.generation.wrapping_add(1);
        }
        self.spill_store = spill_store;
        self.spill_session_key = session_key;
    }
    /// Look a message id up by its full copy: the ledger wins over the live
    /// conversation (which may hold only a collapsed stub). `None` when the ref
    /// is neither in the (bounded) ledger nor the live conversation. Shared by
    /// every resolver so the ledger-then-live precedence stays in one place.
    fn lookup(&self, message_id: &str) -> Option<&Message> {
        self.ledger.get(message_id).or_else(|| {
            super::uds_session::position_by_wire_id(&self.messages, message_id)
                .map(|i| &self.messages[i])
        })
    }
    /// Resolve a message id to its full copy. See [`Self::lookup`].
    #[cfg(test)]
    pub fn resolve(&self, message_id: &str) -> Option<&Message> {
        self.lookup(message_id)
    }
    /// Prepare a `get_message` lookup result. Collapsed messages that carry a
    /// spill id are returned as a deferred recall so callers do not hold the
    /// snapshot read lock across spill-store I/O.
    pub fn resolve_for_get_message(&self, message_id: &str) -> GetMessageResolution {
        let Some(msg) = self.lookup(message_id) else {
            return GetMessageResolution::NotFound;
        };
        if !msg.is_collapsed {
            return GetMessageResolution::Found(msg.clone());
        }
        let Some(spill_id) = msg.spill_id.clone() else {
            return GetMessageResolution::Found(msg.clone());
        };
        let Some(spill_store) = self.spill_store.clone() else {
            return GetMessageResolution::Found(msg.clone());
        };
        GetMessageResolution::Recall {
            stub: msg.clone(),
            spill_store,
            session_key: self.spill_session_key.clone(),
            spill_id,
            generation: self.generation,
        }
    }
    pub fn recall_is_current(&self, recall: &RecallIdentity) -> bool {
        self.generation == recall.generation
            && self.spill_session_key == recall.session_key
            && self.lookup(&recall.message_id).is_some_and(|m| {
                m.is_collapsed && m.spill_id.as_deref() == Some(recall.spill_id.as_str())
            })
    }
    pub fn clear(&mut self) -> LedgerAdvance {
        self.messages.clear();
        self.ledger.clear();
        self.ledger_order.clear();
        self.frontier.clear();
        self.ledger_bytes = 0;
        self.generation = self.generation.wrapping_add(1);
        self.epoch = self.epoch.wrapping_add(1);
        LedgerAdvance {
            epoch: self.epoch,
            rev: self.rev,
            changed: true,
        }
    }
    /// Reset the snapshot to exactly `messages`: drop the whole prior ledger
    /// (so refs from a replaced/truncated conversation stop resolving) and then
    /// re-seed live + ledger from the new set. Used by same-session TRUNCATE ops
    /// (rewind_to) so old refs cannot leak full content out-of-band while the
    /// surviving messages stay resolvable (#1060 review round 4). Ops that also
    /// change the spill namespace (new_session, resume_session) use
    /// [`Self::reset_to_with_spill_store`] instead.
    pub fn reset_to(&mut self, messages: &[Message]) -> LedgerAdvance {
        let advance = self.clear();
        let publish = self.publish(messages);
        LedgerAdvance {
            epoch: self.epoch,
            rev: self.rev,
            changed: advance.changed || publish.changed,
        }
    }
    pub fn reset_to_with_spill_store(
        &mut self,
        messages: &[Message],
        spill_store: Option<Arc<dyn ContextSpillStore>>,
        session_key: String,
    ) -> LedgerAdvance {
        let advance = self.clear();
        self.spill_store = spill_store;
        self.spill_session_key = session_key;
        let publish = self.publish(messages);
        LedgerAdvance {
            epoch: self.epoch,
            rev: self.rev,
            changed: advance.changed || publish.changed,
        }
    }
    pub fn from_messages(messages: Vec<Message>) -> Self {
        let mut s = Self::default();
        let _ = s.publish(&messages);
        s
    }
    pub fn sync_json(&self, epoch: u64, since_rev: u64) -> serde_json::Value {
        let resync = epoch != self.epoch
            || self.frontier.front().is_some_and(|(r, _)| since_rev < *r)
            || self
                .frontier
                .iter()
                .any(|(_, id)| self.lookup(id).is_none());
        if resync {
            let mut data = messages_page_json(&self.messages, HISTORY_PAGE_SIZE, None);
            if let Some(obj) = data.as_object_mut() {
                obj.insert("epoch".into(), serde_json::json!(self.epoch));
                obj.insert("rev".into(), serde_json::json!(self.rev));
                obj.insert("nextRev".into(), serde_json::Value::Null);
                obj.insert("caughtUp".into(), serde_json::json!(true));
                obj.insert("resync".into(), serde_json::json!(true));
            }
            return data;
        }
        let candidates: Vec<(u64, &Message)> = self
            .frontier
            .iter()
            .filter(|(rev, _)| *rev > since_rev)
            .filter_map(|(rev, id)| self.lookup(id).map(|m| (*rev, m)))
            .collect();
        let mut selected: Vec<(u64, serde_json::Value)> = Vec::new();
        let mut used = 0usize;
        let mut next_rev = None;
        for (rev, msg) in &candidates {
            let value = sync_message_json(msg);
            let sz = serde_json::to_vec(&value)
                .map(|v| v.len())
                .unwrap_or(usize::MAX)
                + 1;
            if used.saturating_add(sz) > super::uds_session::HISTORY_PAGE_JSON_BUDGET {
                // If even the first bounded representation is too large for a
                // sync frame, do not emit an over-cap success that the
                // transport will replace with an unstructured frame-limit
                // error. Instead return a small sync page that advances through
                // the oversized ledger revision; the message remains available
                // through get_message/get_messages summary paths.
                next_rev = Some(*rev);
                break;
            }
            used = used.saturating_add(sz);
            selected.push((*rev, value));
        }
        if next_rev.is_none() && candidates.len() > selected.len() {
            next_rev = selected.last().map(|(newest, _)| *newest);
        }
        serde_json::json!({
            "epoch": self.epoch,
            "rev": self.rev,
            "messages": selected.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
            "nextRev": next_rev,
            "caughtUp": next_rev.is_none(),
            "resync": false,
        })
    }
}

pub(crate) enum GetMessageResolution {
    Found(Message),
    Recall {
        stub: Message,
        spill_store: Arc<dyn ContextSpillStore>,
        session_key: String,
        spill_id: String,
        generation: u64,
    },
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecallIdentity {
    pub message_id: String,
    pub session_key: String,
    pub spill_id: String,
    pub generation: u64,
}

impl GetMessageResolution {
    /// Finish the best-effort lookup outside the snapshot read-lock. Spill
    /// misses/errors gracefully fall back to the live collapsed stub. A
    /// successful read carries its captured identity so the caller can reject
    /// it if a concurrent lifecycle operation replaced the history.
    pub async fn into_message(self) -> ResolvedGetMessage {
        match self {
            Self::Found(message) => ResolvedGetMessage {
                message: Some(message),
                recalled: None,
            },
            Self::Recall {
                mut stub,
                spill_store,
                session_key,
                spill_id,
                generation,
            } => {
                let identity = RecallIdentity {
                    message_id: stub.id().to_string(),
                    session_key: session_key.clone(),
                    spill_id: spill_id.clone(),
                    generation,
                };
                match spill_store.recall(&session_key, &spill_id).await {
                    Ok(Some(entry)) => {
                        stub.content = entry.content;
                        stub.is_collapsed = false;
                        stub.spill_id = None;
                        ResolvedGetMessage {
                            message: Some(stub),
                            recalled: Some(identity),
                        }
                    }
                    Ok(None) | Err(_) => ResolvedGetMessage {
                        message: Some(stub),
                        recalled: Some(identity),
                    },
                }
            }
            Self::NotFound => ResolvedGetMessage {
                message: None,
                recalled: None,
            },
        }
    }
}

pub(crate) struct ResolvedGetMessage {
    pub message: Option<Message>,
    pub recalled: Option<RecallIdentity>,
}

pub(crate) type ConversationSnapshot =
    std::sync::Arc<tokio::sync::RwLock<ConversationSnapshotData>>;

/// Resolve a message without holding the snapshot lock across spill I/O. If a
/// lifecycle operation changes the history during that I/O, discard the stale
/// result and retry against the new snapshot. This validation applies to both
/// hits and fallback stubs: neither may be returned from an old session.
///
/// Lifecycle ops are serialized on the dispatch loop, so at most a handful of
/// replacements can race one lookup; the retry cap only backstops pathological
/// churn. On exhausting it we resolve once more against the *current* snapshot
/// and return its live view WITHOUT another spill read — never a stale result,
/// since that view is read directly from the now-current history.
pub(crate) async fn resolve_get_message(
    snapshot: &ConversationSnapshot,
    message_id: &str,
) -> Option<Message> {
    const MAX_RECALL_RETRIES: usize = 8;
    for _ in 0..MAX_RECALL_RETRIES {
        let resolution = { snapshot.read().await.resolve_for_get_message(message_id) };
        let resolved = resolution.into_message().await;
        let Some(recall) = &resolved.recalled else {
            return resolved.message;
        };
        if snapshot.read().await.recall_is_current(recall) {
            return resolved.message;
        }
    }
    // Final fallback: return the current live message (the collapsed stub if it
    // is still present) without deferring another recall, guaranteeing both
    // termination and current-session correctness.
    match snapshot.read().await.resolve_for_get_message(message_id) {
        GetMessageResolution::Found(message) => Some(message),
        GetMessageResolution::Recall { stub, .. } => Some(stub),
        GetMessageResolution::NotFound => None,
    }
}

fn sync_message_json(msg: &Message) -> serde_json::Value {
    message_to_json_for_history_page(msg)
}

pub(crate) type SessionStatsSnapshot =
    std::sync::Arc<tokio::sync::RwLock<crate::interface::cli::protocol::SessionStats>>;

/// Refresh every busy-child snapshot (state / conversation / session_stats /
/// extensions) at once. Called per INNER turn inside the drain/nudge loop so a
/// busy `get_state` mid-workflow tracks progress + message count step-by-step,
/// instead of being frozen at the pre-turn (often initial) view until the whole
/// dispatched command returns (#899). The `snapshot: true` staleness marker is
/// retained — a busy snapshot may still lag the in-flight turn by design, but it
/// must not lag by an entire workflow.
pub(super) async fn refresh_busy_snapshots(ctx: &DispatchCtx<'_>) {
    refresh_conversation_snapshot(ctx).await;
    refresh_state_snapshot(ctx).await;
    refresh_session_stats_snapshot(ctx).await;
    refresh_tool_catalogue_snapshot(ctx).await;
}

pub(super) async fn refresh_conversation_snapshot(ctx: &DispatchCtx<'_>) {
    let mut snap = ctx.conversation_snapshot.write().await;
    snap.set_spill_store(ctx.agent.spill_store().cloned(), ctx.session_key.clone());
    let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
    let advance = snap.publish(&visible_messages);
    drop(snap);
    if advance.changed
        && let Some(tx) = ctx.broadcast_tx.as_ref()
    {
        let _ = tx.send(
            serde_json::json!({"type":"ledger_advanced","epoch":advance.epoch,"rev":advance.rev})
                .to_string()
                + "\n",
        );
    }
}

pub(super) async fn refresh_state_snapshot(ctx: &DispatchCtx<'_>) {
    let workflow = ctx.workflow_state.as_ref().and_then(|ws| {
        ws.lock().ok().map(|engine| {
            let mut value = serde_json::to_value(engine.snapshot(true)).unwrap_or_default();
            if let Some(config) = &ctx.workflow_config {
                value["automation"] = serde_json::json!({
                    "autoContinue": config.auto_continue,
                    "completionNudge": config.completion_nudge,
                });
            }
            value
        })
    });
    let visible_message_count = user_visible_messages(ctx.messages, ctx.system_prompt).len();
    let state = ctx.session.state_snapshot(
        visible_message_count,
        workflow,
        ctx.agent.max_context_tokens(),
        ctx.agent.effort().map(|l| l.as_str().to_string()),
    );
    let mut snap = ctx.state_snapshot.write().await;
    *snap = state;
}

pub(super) async fn refresh_session_stats_snapshot(ctx: &DispatchCtx<'_>) {
    let visible_messages = user_visible_messages(ctx.messages, ctx.system_prompt);
    let stats = compute_session_stats_with_usage(
        ctx.session_key,
        &visible_messages,
        ctx.session.usage_snapshot(),
        ctx.session.context_tokens(),
        ctx.agent.max_context_tokens(),
    );
    let mut snap = ctx.session_stats_snapshot.write().await;
    *snap = stats;
}

pub(super) async fn refresh_tool_catalogue_snapshot(ctx: &DispatchCtx<'_>) {
    let mut snap = ctx.tool_catalogue_snapshot.write().await;
    *snap = ctx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .map(|entry| serde_json::to_value(entry).unwrap_or_default())
        .collect();
}

/// Build the connect-time `get_messages` snapshot line a BUSY child pushes.
///
/// The `data.snapshot` marker tells callers the data may lag the in-flight turn
/// (a live dispatch-loop reply has no such marker) (#842). History uses the same
/// byte-bounded page shaping as the live query path so a BUSY child with one
/// oversized recent message still returns a recoverable summary (#1107).
pub(crate) fn build_get_messages_line(messages: &[Message]) -> String {
    let mut data = messages_page_json(messages, HISTORY_PAGE_SIZE, None);
    if let Some(obj) = data.as_object_mut() {
        if obj.get("before").is_some_and(serde_json::Value::is_null) {
            obj.remove("before");
        }
        obj.insert("snapshot".into(), serde_json::json!(true));
    }
    let mut line = AgentEvent::ok(None, "get_messages", Some(data)).to_json_line();
    debug_assert!(line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    line.push('\n');
    line
}

/// Build the connect-time `get_subagents` snapshot line a BUSY child pushes.
///
/// The `SubagentRegistry` is an `Arc<Mutex<…>>` independent of the dispatch
/// loop's exclusive `&mut messages` borrow, so a busy child can serve its
/// current registry view off the turn (#874). A `None` registry yields an empty
/// subagents list (matching [`build_subagent_info_list`]'s contract), not an
/// error. The `data.snapshot` marker tells callers the data may lag the
/// in-flight turn, consistent with the #842 snapshot markers.
pub(crate) fn build_get_subagents_line(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) -> String {
    let data = serde_json::json!({
        "subagents": serde_json::to_value(super::protocol::build_subagent_info_list(registry))
            .unwrap_or_default(),
        "snapshot": true,
    });
    let ev = AgentEvent::ok(None, "get_subagents", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) fn build_get_session_stats_line(
    stats: &crate::interface::cli::protocol::SessionStats,
) -> String {
    let mut data = serde_json::to_value(stats).unwrap_or_default();
    if let Some(obj) = data.as_object_mut() {
        obj.insert("snapshot".to_string(), serde_json::json!(true));
    }
    let ev = AgentEvent::ok(None, "get_session_stats", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) fn build_get_tool_catalogue_line(tools: &[serde_json::Value]) -> String {
    let data = serde_json::json!({
        "tools": tools,
        "snapshot": true,
    });
    let ev = AgentEvent::ok(None, "get_tool_catalogue", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) struct BusySnapshotSources<'a> {
    pub state: &'a StateSnapshot,
    pub conversation: &'a ConversationSnapshot,
    pub session_stats: &'a SessionStatsSnapshot,
    pub tool_catalogue: &'a crate::interface::cli::uds_extensions::ToolCatalogueSnapshot,
    pub subagents: &'a Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub workflow: &'a Option<crate::interface::shared::WorkflowStateHandle>,
    pub execution: &'a super::uds_execution_state::ExecutionStateHandle,
}

pub(crate) async fn busy_connect_snapshot_lines(sources: BusySnapshotSources<'_>) -> [String; 5] {
    let BusySnapshotSources {
        state: state_snapshot,
        conversation: conversation_snapshot,
        session_stats: session_stats_snapshot,
        tool_catalogue: tool_catalogue_snapshot,
        subagents: subagent_registry,
        workflow: workflow_state,
        execution: execution_state,
    } = sources;
    let state_line = {
        // Release the async snapshot lock before taking either sync mutex.
        let live = state_snapshot.read().await.clone();
        build_busy_get_state_line(&live, workflow_state, execution_state)
    };
    let messages_line = {
        let snap = conversation_snapshot.read().await;
        build_get_messages_line(&snap.messages)
    };
    let stats_line = {
        let stats = session_stats_snapshot.read().await;
        build_get_session_stats_line(&stats)
    };
    let extensions_line = {
        let tools = tool_catalogue_snapshot.read().await;
        build_get_tool_catalogue_line(&tools)
    };
    [
        state_line,
        messages_line,
        build_get_subagents_line(subagent_registry),
        stats_line,
        extensions_line,
    ]
}

pub(crate) fn build_busy_get_state_line(
    state: &SessionState,
    workflow_state: &Option<crate::interface::shared::WorkflowStateHandle>,
    execution_state: &super::uds_execution_state::ExecutionStateHandle,
) -> String {
    let mut live = state.clone();
    // Projection and revision come from one workflow critical section. Drop it
    // before execution to keep a single, non-nested lock order.
    let workflow_revision = if let Some(workflow) = workflow_state {
        let engine = workflow
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let revision = engine.revision();
        live.workflow = Some(serde_json::to_value(engine.snapshot(true)).unwrap_or_default());
        revision
    } else {
        0
    };
    let mut execution = execution_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    live.generation = execution.observe_visible_revisions(live.generation, workflow_revision);
    live.message_count = execution.message_count();
    live.execution = Some(execution.snapshot());
    drop(execution);
    build_get_state_line_live(&live, &None, true)
}

#[cfg(test)]
pub(crate) fn build_get_state_line(state: &SessionState) -> String {
    build_get_state_line_with_streaming(state, state.is_streaming)
}

#[cfg(test)]
pub(crate) fn build_get_state_line_with_streaming(
    state: &SessionState,
    is_streaming: bool,
) -> String {
    let mut state = state.clone();
    state.is_streaming = is_streaming;
    state.sync = 1;
    let ev = AgentEvent::ok(
        None,
        "get_state",
        Some(super::uds_state_projection::slim_state_projection(&state)),
    );
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

/// Busy `get_state` line with the LIVE workflow engine overlaid onto the frozen
/// session snapshot (#914). The periodic `state_snapshot` only refreshes at turn
/// boundaries, but workflow steps are checked off mid-turn via the `workflow`
/// tool, so a busy `get_state` served from the frozen snapshot only ever shows
/// `0/N` (pre-turn) or `N/N` (post-turn). The engine is an `Arc<Mutex<…>>`
/// independent of the dispatch loop's `&mut messages`; we lock it briefly and
/// synchronously (no `.await` held) to read its current snapshot, mirroring how
/// `refresh_state_snapshot` serializes it. Automation flags are preserved from
/// the frozen snapshot (they come from workflow config, not the engine).
pub(crate) fn build_get_state_line_live(
    state: &SessionState,
    workflow_state: &Option<crate::interface::shared::WorkflowStateHandle>,
    is_streaming: bool,
) -> String {
    let mut state = state.clone();
    state.is_streaming = is_streaming;
    state.sync = 1;
    if let Some(ws) = workflow_state {
        let engine = ws.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut live = serde_json::to_value(engine.snapshot(true)).unwrap_or_default();
        if let Some(auto) = state
            .workflow
            .as_ref()
            .and_then(|w| w.get("automation"))
            .cloned()
        {
            live["automation"] = auto;
        }
        state.workflow = Some(live);
    }
    let data = super::uds_state_projection::slim_state_projection(&state);
    let ev = AgentEvent::ok(None, "get_state", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}
