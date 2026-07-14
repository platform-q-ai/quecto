use std::sync::Arc;

use crate::domain::message::Message;
use crate::domain::session::ContextSpillStore;

use serde_json::value::RawValue;

use super::protocol::{AgentEvent, SessionState};
use super::uds::DispatchCtx;
use super::uds_session::{MessageView, compute_session_stats_with_usage};

pub(crate) type StateSnapshot = std::sync::Arc<tokio::sync::RwLock<SessionState>>;

/// Memory budgets for the id-addressable ledger. Quecto sessions run for weeks,
/// so the ledger must NOT grow without bound. Eviction (oldest-first) triggers
/// on WHICHEVER cap is hit: a content-byte budget for large-payload messages,
/// AND an entry-count cap so a flood of tiny/empty/tool-metadata messages — each
/// of which adds little content but a real per-entry cost (two id-string copies
/// for the map key + order deque, plus the cloned Message/ToolCall structs) —
/// cannot accumulate unbounded. A ref whose full copy has been evicted resolves
/// best-effort (get_message returns "not found", or a collapsed stub if the
/// message is still live); end-of-turn refs point at recent messages that
/// clients resolve promptly, so eviction is invisible in practice (#1060 review
/// r4, finding 2 — unbounded ledger).
const LEDGER_MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const LEDGER_MAX_ENTRIES: usize = 8192;

/// Fixed per-entry overhead folded into the byte budget so a zero/tiny-content
/// message still consumes budget: it covers the id `String` stored twice (map
/// key + order deque, ~2×UUID) plus the owned Message/ToolCall struct footprint.
const LEDGER_ENTRY_OVERHEAD: usize = 256;

/// Approximate owned in-memory size of a ledger entry, for byte-budgeting —
/// content + tool payloads + the fixed per-entry overhead above.
fn message_bytes(m: &Message) -> usize {
    LEDGER_ENTRY_OVERHEAD
        + m.content.len()
        + m.tool_calls
            .iter()
            .map(|tc| tc.arguments.len() + tc.name.len() + tc.id.len())
            .sum::<usize>()
        + m.tool_call_id.as_ref().map_or(0, |s| s.len())
        + m.tool_name.as_ref().map_or(0, |s| s.len())
}

/// The busy-path conversation snapshot: the live (post-prune) conversation for
/// `get_messages` inspection, PLUS a BYTE-BOUNDED id→message ledger.
///
/// #1060 review 1a: end-of-turn `messageRefs` are the ids of the run's
/// `appended_messages` — full copies the context ladder never demotes. The live
/// conversation, however, can drop or collapse-in-place those messages to fit
/// the LLM budget, so a bare `get_message` against it can return "not found" or
/// a stub for a ref that was just emitted. The ledger keeps full copies so a ref
/// stays resolvable across pruning — but capped by [`LEDGER_MAX_BYTES`] AND
/// [`LEDGER_MAX_ENTRIES`], oldest-first, so it cannot grow unbounded over a
/// weeks-long session.
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
}

impl ConversationSnapshotData {
    /// Fold one message into the ledger under the byte budget. `overwrite`
    /// replaces an existing (possibly collapsed) entry with a full copy; without
    /// it, an earlier full copy is kept (publish must not clobber it).
    fn remember(&mut self, m: &Message, overwrite: bool) {
        let id = m.id().to_string();
        let already = self.ledger.contains_key(&id);
        if already && !overwrite {
            return;
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
            }
        }
    }

    /// Replace the live view and fold its messages into the ledger WITHOUT
    /// overwriting existing entries — a full copy recorded earlier must survive
    /// a later in-place collapse of the live message.
    pub fn publish(&mut self, messages: &[Message]) {
        for m in messages {
            self.remember(m, false);
        }
        self.messages = messages.to_vec();
    }

    /// Record authoritative FULL copies (the run's un-demoted `appended_messages`),
    /// overwriting any earlier possibly-collapsed entry.
    pub fn record_full(&mut self, messages: &[Message]) {
        for m in messages {
            self.remember(m, true);
        }
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
            self.messages
                .iter()
                .find(|m| m.id().to_string() == message_id)
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

    /// Whether a deferred spill result still belongs to the currently
    /// published history. The message id + spill id checks matter even within
    /// one session: rewind/history replacement can reuse session-local spill
    /// identifiers while changing which stable message owns them.
    pub fn recall_is_current(&self, recall: &RecallIdentity) -> bool {
        self.generation == recall.generation
            && self.spill_session_key == recall.session_key
            && self.lookup(&recall.message_id).is_some_and(|m| {
                m.is_collapsed && m.spill_id.as_deref() == Some(recall.spill_id.as_str())
            })
    }

    /// Clear both the live snapshot and the full-message lookup ledger. Explicit
    /// history lifecycle operations (for example `clear_history`) must make old
    /// refs unresolvable rather than retaining full content out-of-band.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.ledger.clear();
        self.ledger_order.clear();
        self.ledger_bytes = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Reset the snapshot to exactly `messages`: drop the whole prior ledger
    /// (so refs from a replaced/truncated conversation stop resolving) and then
    /// re-seed live + ledger from the new set. Used by same-session TRUNCATE ops
    /// (rewind_to) so old refs cannot leak full content out-of-band while the
    /// surviving messages stay resolvable (#1060 review round 4). Ops that also
    /// change the spill namespace (new_session, resume_session) use
    /// [`Self::reset_to_with_spill_store`] instead.
    pub fn reset_to(&mut self, messages: &[Message]) {
        self.clear();
        self.publish(messages);
    }

    /// Atomically replace both history and the namespace used to recall its
    /// collapsed messages. Resume/new-session paths must not expose new
    /// messages paired with the previous session's spill key, even briefly.
    pub fn reset_to_with_spill_store(
        &mut self,
        messages: &[Message],
        spill_store: Option<Arc<dyn ContextSpillStore>>,
        session_key: String,
    ) {
        self.clear();
        self.spill_store = spill_store;
        self.spill_session_key = session_key;
        self.publish(messages);
    }

    /// Seed a snapshot from an initial conversation (test/lifecycle convenience).
    pub fn from_messages(messages: Vec<Message>) -> Self {
        let mut s = Self::default();
        s.publish(&messages);
        s
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
    refresh_extension_snapshot(ctx).await;
}

pub(super) async fn refresh_conversation_snapshot(ctx: &DispatchCtx<'_>) {
    let mut snap = ctx.conversation_snapshot.write().await;
    snap.set_spill_store(ctx.agent.spill_store().cloned(), ctx.session_key.clone());
    snap.publish(ctx.messages);
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
    let state = ctx.session.state_snapshot(
        ctx.messages.len(),
        workflow,
        ctx.agent.max_context_tokens(),
        ctx.agent.effort().map(|l| l.as_str().to_string()),
    );
    let mut snap = ctx.state_snapshot.write().await;
    *snap = state;
}

pub(super) async fn refresh_session_stats_snapshot(ctx: &DispatchCtx<'_>) {
    let stats = compute_session_stats_with_usage(
        ctx.session_key,
        ctx.messages,
        ctx.session.usage_snapshot(),
        ctx.session.context_tokens(),
        ctx.agent.max_context_tokens(),
    );
    let mut snap = ctx.session_stats_snapshot.write().await;
    *snap = stats;
}

pub(super) async fn refresh_extension_snapshot(ctx: &DispatchCtx<'_>) {
    let mut snap = ctx.extension_snapshot.write().await;
    *snap = crate::interface::cli::uds_extensions::build_extension_list(ctx);
}

/// Byte budget for a connect-time `get_messages` snapshot line. Kept just under
/// the parent's per-line read cap (`SUBAGENT_RESPONSE_MAX_FRAME_PAYLOAD_BYTES` = 8 MiB in
/// `subagent_registry`) with headroom for the response envelope, so an oversized
/// history is tailed to fit rather than making the parent's whole call error
/// ("line exceeded size limit") on a busy child (#842).
pub(super) const SNAPSHOT_MESSAGES_BUDGET_BYTES: usize =
    quecto_line_io::PROTOCOL_LINE_CAP_BYTES - 4096;

/// Build the connect-time `get_messages` snapshot line a BUSY child pushes.
///
/// The `data.snapshot` marker tells callers the data may lag the in-flight turn
/// (a live dispatch-loop reply has no such marker) (#842). When the serialized
/// history would exceed [`SNAPSHOT_MESSAGES_BUDGET_BYTES`], the OLDEST messages
/// are dropped so the most recent (the inspection target) still arrive, with
/// `data.trimmed` set — counted/tail readers slice this further on the parent
/// side, so a tail is exactly what they want. A single message that alone exceeds
/// the budget cannot be returned under the parent's read cap, so it is dropped
/// too (yielding an empty `trimmed` snapshot rather than erroring the call).
pub(crate) fn build_get_messages_line(messages: &[Message]) -> String {
    // Serialize each message EXACTLY ONCE into an owned `RawValue`; its `.get()`
    // length is used for byte-budgeting and the same bytes are re-emitted
    // verbatim in the final line (no second serialization, no Value tree) (#994).
    let mut raws: Vec<Box<RawValue>> = messages
        .iter()
        .map(|m| {
            serde_json::value::to_raw_value(&MessageView(m)).unwrap_or_else(|_| {
                RawValue::from_string("null".to_string()).expect("null literal")
            })
        })
        .collect();

    // Accumulate from the newest message backwards until the next (older) one
    // would breach the budget; `start` is the index of the oldest kept message.
    let mut total = 0usize;
    let mut start = raws.len();
    for (i, rv) in raws.iter().enumerate().rev() {
        let sz = rv.get().len() + 1; // +1 for the array separator
        if total + sz > SNAPSHOT_MESSAGES_BUDGET_BYTES {
            break;
        }
        total += sz;
        start = i;
    }
    let trimmed = start > 0;
    // `split_off` moves the kept tail out in place — no slice clone (#994).
    let kept = raws.split_off(start);

    let line_body = GetMessagesSnapshot::Response {
        command: "get_messages",
        success: true,
        data: GetMessagesData {
            messages: &kept,
            snapshot: true,
            trimmed,
        },
    };
    let mut line =
        serde_json::to_string(&line_body).expect("get_messages snapshot is always serializable");
    line.push('\n');
    line
}

/// Serializes byte-identically (modulo key order) to
/// `AgentEvent::ok(None, "get_messages", Some(data))`, but embeds the
/// pre-serialized message `RawValue`s directly so each message is serialized at
/// most once (#994).
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GetMessagesSnapshot<'a> {
    Response {
        command: &'a str,
        success: bool,
        data: GetMessagesData<'a>,
    },
}

#[derive(serde::Serialize)]
struct GetMessagesData<'a> {
    messages: &'a [Box<RawValue>],
    snapshot: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    trimmed: bool,
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

pub(crate) fn build_get_extensions_line(extensions: &[serde_json::Value]) -> String {
    let data = serde_json::json!({
        "extensions": extensions,
        "snapshot": true,
    });
    let ev = AgentEvent::ok(None, "get_extensions", Some(data));
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}

pub(crate) async fn busy_connect_snapshot_lines(
    state_snapshot: &StateSnapshot,
    conversation_snapshot: &ConversationSnapshot,
    session_stats_snapshot: &SessionStatsSnapshot,
    extension_snapshot: &crate::interface::cli::uds_extensions::ExtensionSnapshot,
    subagent_registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    workflow_state: &Option<crate::interface::shared::WorkflowStateHandle>,
) -> [String; 5] {
    let state_line = {
        let snap = state_snapshot.read().await;
        // #914: overlay the LIVE workflow engine onto the (turn-boundary) frozen
        // snapshot so a busy `get_state` reports mid-turn step progress, not just
        // 0/N (pre-turn) or N/N (post-turn). The engine `Mutex` is independent of
        // the dispatch loop's `&mut messages`, so this is safe to read mid-turn.
        build_get_state_line_live(&snap, workflow_state, true)
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
        let extensions = extension_snapshot.read().await;
        build_get_extensions_line(&extensions)
    };
    [
        state_line,
        messages_line,
        build_get_subagents_line(subagent_registry),
        stats_line,
        extensions_line,
    ]
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
    let ev = AgentEvent::ok(
        None,
        "get_state",
        Some(serde_json::to_value(state).unwrap_or_default()),
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
    if let Some(ws) = workflow_state {
        if let Ok(engine) = ws.lock() {
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
    }
    let ev = AgentEvent::ok(
        None,
        "get_state",
        Some(serde_json::to_value(state).unwrap_or_default()),
    );
    let mut line = ev.to_json_line();
    line.push('\n');
    line
}
