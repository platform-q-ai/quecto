use super::protocol::{SessionState, SessionStats, TokenStats};
use crate::application::agent_loop::UsageTotals;
/// UDS session state — in-memory tracker and statistics for an active UDS connection.
use crate::domain::message::{Message, Role};
// ─── Session state tracker ────────────────────────────────────────────────────
/// In-memory state for an active UDS session.
#[path = "uds_session_notify.rs"]
mod uds_session_notify;
pub use uds_session_notify::NotificationEnqueueOutcome;
/// The tracker holds no session key (D10 #1979): the active session's typed
/// identity is the one owner of `sessionKey`, and every presenter that
/// reports it is handed the key by its caller from that identity.
#[derive(Debug)]
pub struct AgentSession {
    pub(crate) commander: super::agent_commander_wiring::SessionCommander,
    model: String,
    admission_warnings: Vec<crate::domain::state_snapshot::AdmissionBindingWarning>,
    runtime_store: Option<crate::application::ports::RuntimeSnapshotStore>,
    streaming: bool,
    pub(crate) automatic_turns_allowed: bool,
    /// Why automatic turns are off, and at which control generation (#1721).
    suspension: Option<TurnSuspension>,
    /// Latest swarm control generation this session has seen (control
    /// receipts, the startup probe); dates a suspension when it happens.
    last_control_generation: Option<u64>,
    /// A resume re-armed this session: it owes the run one turn to continue
    /// its interrupted work, taken by the next runnable wake.
    pending_resume_turn: bool,
    generation: u64,
    /// Cumulative provider-reported usage for this in-memory UDS session.
    usage: SessionUsage,
    /// Latest user-facing context occupancy reported by the agent loop:
    /// provider-truth when available, estimate fallback otherwise.
    context_tokens: usize,
    /// `VecDeque` supports O(1) push_back (enqueue) and push_front (prepend/steer).
    pending: std::collections::VecDeque<PendingMessage>,
    control_receipts: Vec<super::protocol::ControlReceipt>,
    last_subagent_notification: std::collections::HashMap<String, u64>,
    last_failure_notifications: std::collections::HashMap<String, String>,
    repeated_failure_notifications: u64,
    /// Subagent notes that arrived while `pending` was full (#1082 review
    /// round 2). Retained here — with their dedupe sequence recorded — and
    /// appended by [`Self::drain_pending`], so supervision-critical notes
    /// survive a saturated queue end-to-end instead of being dropped.
    overflow_notifications: std::collections::VecDeque<PendingMessage>,
}
#[path = "uds_session_admission.rs"]
mod admission_warnings;
#[path = "uds_session_suspension.rs"]
mod suspension;
pub use suspension::{SuspensionCause, TurnSuspension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingMessage {
    User(String),
    Control {
        id: String,
        command: String,
        content: String,
    },
    /// A harness-generated prompt (a swarm wake nudge parked behind a steer):
    /// runs like a user prompt but never re-arms a suspended member (#1712).
    Automatic(String),
    SubagentNotification {
        agent_id: String,
        sequence: u64,
        content: String,
        /// `true` for a successful completion; `false` for an errored/exited
        /// note. Only completions are eligible for coalescing — failures keep
        /// their individual note so the error detail is never dropped (#894).
        is_completion: bool,
    },
    /// A SINGLE informational note summarizing a batch of sub-agent completions
    /// that drained together at one idle boundary (#894). Built by
    /// [`coalesce_pending`]; the body already lists the agent names.
    CoalescedSubagentNotification {
        content: String,
    },
}
impl PendingMessage {
    pub fn user(content: String) -> Self {
        Self::User(content)
    }
    pub fn subagent_notification(
        agent_id: String,
        sequence: u64,
        content: String,
        is_completion: bool,
    ) -> Self {
        Self::SubagentNotification {
            agent_id,
            sequence,
            content,
            is_completion,
        }
    }
    /// Convert to the message injected into the parent conversation.
    ///
    /// Sub-agent notes are `Role::User`, not `Role::System` (#1338). A system
    /// message is not a conversational turn: the Anthropic provider hoists it
    /// into the top-level `system` field, so the request ended on the parent's
    /// own last assistant message and the model merely continued its previous
    /// answer ("OK, will do") instead of acting on the note. It also rewrote
    /// the cached system block every time, missing the prompt cache for the
    /// whole request. The `<subagent_notification>` wrapper marks the note as
    /// harness-injected so clients can render it distinctly from user input.
    pub fn into_message(self) -> Message {
        match self {
            Self::User(content) | Self::Control { content, .. } | Self::Automatic(content) => {
                Message::user(content)
            }
            Self::SubagentNotification {
                agent_id,
                sequence,
                content,
                ..
            } => Message::user(format!(
                "<subagent_notification source=\"spawn_tool\" agent_id=\"{}\" sequence=\"{}\">\n{}\n</subagent_notification>",
                escape_attr(&agent_id),
                sequence,
                escape_text(&content)
            )),
            Self::CoalescedSubagentNotification { content } => Message::user(format!(
                "<subagent_notification source=\"spawn_tool\" coalesced=\"true\">\n{}\n</subagent_notification>",
                escape_text(&content)
            )),
        }
    }
}
/// Maximum number of agent names listed verbatim in a coalesced completion note
/// before the remainder is summarized as a `(+M more)` tail (#894).
const COALESCE_NAME_CAP: usize = 10;
/// Collapse a batch of pending messages drained together so that MORE THAN ONE
/// sub-agent completion note surfaces as a SINGLE informational summary (#894).
///
/// When a burst of children finish during one busy parent turn, every note is
/// buffered (deferred to the idle boundary by #816) and drains together here. K
/// separate "…ready for inspection" notes would send the parent into a catch-up
/// loop; instead they collapse to one ambient note listing the names (capped),
/// so the parent decides if/when to inspect.
///
/// A single completion (or zero) passes through untouched — a clean one-line
/// note. Non-notification pending messages (steer/follow-up) are preserved in
/// order; the lone coalesced note takes the position of the first notification.
///
/// Only SUCCESSFUL completions are coalesced. Errored/Exited notifications carry
/// a failure signal and inline error detail that must never be laundered into an
/// idle turn-end summary, so they always pass through as their own individual note
/// (#894). A mixed batch therefore yields one coalesced completion summary plus
/// each failure note kept verbatim.
pub fn coalesce_pending(pending: Vec<PendingMessage>) -> Vec<PendingMessage> {
    let names: Vec<&str> = pending
        .iter()
        .filter_map(|m| match m {
            PendingMessage::SubagentNotification {
                agent_id,
                is_completion: true,
                ..
            } => Some(agent_id.as_str()),
            _ => None,
        })
        .collect();
    if names.len() <= 1 {
        return pending;
    }
    let coalesced = PendingMessage::CoalescedSubagentNotification {
        content: coalesced_note_text(&names),
    };
    let mut out = Vec::with_capacity(pending.len() - names.len() + 1);
    let mut inserted = false;
    for msg in pending {
        match msg {
            // Only completions collapse; failures (is_completion=false) fall
            // through to the catch-all and keep their own note.
            PendingMessage::SubagentNotification {
                is_completion: true,
                ..
            } => {
                if !inserted {
                    out.push(coalesced.clone());
                    inserted = true;
                }
            }
            other => out.push(other),
        }
    }
    out
}
/// Build the body of a coalesced completion note: `"N sub-agents ended a turn
/// (status: idle) (a, b, c). …"`, capping the name list at
/// [`COALESCE_NAME_CAP`] with a `(+M more)` tail (#894, wording per #1071).
fn coalesced_note_text(names: &[&str]) -> String {
    let total = names.len();
    let shown = total.min(COALESCE_NAME_CAP);
    let mut list = names[..shown].join(", ");
    if total > shown {
        list.push_str(&format!(" (+{} more)", total - shown));
    }
    format!(
        "{total} sub-agents ended a turn (status: idle) ({list}). \
         Inspect agent_cmd get_messages for each before treating their work as complete."
    )
}
fn escape_attr(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
}
fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
impl AgentSession {
    pub fn new(model: String) -> Self {
        Self {
            commander: None,
            model,
            admission_warnings: Vec::new(),
            runtime_store: None,
            streaming: false,
            automatic_turns_allowed: true,
            suspension: None,
            last_control_generation: None,
            pending_resume_turn: false,
            generation: 1,
            usage: SessionUsage::default(),
            context_tokens: 0,
            pending: std::collections::VecDeque::new(),
            control_receipts: Vec::new(),
            last_subagent_notification: std::collections::HashMap::new(),
            last_failure_notifications: std::collections::HashMap::new(),
            repeated_failure_notifications: 0,
            overflow_notifications: std::collections::VecDeque::new(),
        }
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }
    pub(crate) fn bump_visible_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
    }
    pub fn set_model(&mut self, model: String) {
        if self.model != model {
            self.model = model;
            self.bump_visible_generation();
        }
    }
    /// The loop moved to another session (D7 #1976, D10 #1979): the usage
    /// accumulated for the departed session is cleared and the visible
    /// generation bumps once, exactly as the tracker's own key change did
    /// while it still held a copy. The propagation adapter calls this only
    /// when the identity actually changed.
    pub fn session_changed(&mut self) {
        self.clear_usage();
        self.bump_visible_generation();
    }
    pub fn set_streaming(&mut self, v: bool) {
        if self.streaming != v {
            self.streaming = v;
            self.bump_visible_generation();
        }
    }
    /// Whole-result accumulator kept for tests; production records usage
    /// through `record_usage` on the UDS run path.
    #[cfg(test)]
    pub fn record_agent_result(
        &mut self,
        session_key: &str,
        result: &crate::domain::agent::AgentResult,
    ) {
        self.context_tokens = result.context_tokens;
        self.record_usage(
            session_key,
            UsageTotals::billed(
                result.billed_input_tokens,
                result.billed_output_tokens,
                result.cache_read_tokens,
                result.cache_write_tokens,
                result.cost_micro_usd,
            ),
        );
    }
    pub fn context_tokens(&self) -> usize {
        self.context_tokens
    }
    pub fn set_context_tokens(&mut self, context_tokens: usize) {
        self.context_tokens = context_tokens;
    }
    /// Accumulate the billed usage of one turn; `session_key` is the active
    /// session's key, named by the normalized usage log (#1567).
    pub fn record_usage(&mut self, session_key: &str, usage: UsageTotals) {
        let UsageTotals {
            billed_input_tokens: input_tokens,
            billed_output_tokens: output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            cost_micro_usd,
            ..
        } = usage;
        self.usage.tokens.input = self.usage.tokens.input.saturating_add(input_tokens);
        self.usage.tokens.output = self.usage.tokens.output.saturating_add(output_tokens);
        self.usage.tokens.cache_read = self
            .usage
            .tokens
            .cache_read
            .saturating_add(cache_read_tokens);
        self.usage.tokens.cache_write = self
            .usage
            .tokens
            .cache_write
            .saturating_add(cache_write_tokens);
        self.usage.tokens.total = self
            .usage
            .tokens
            .input
            .saturating_add(self.usage.tokens.output);
        self.usage.cost_micro_usd = self.usage.cost_micro_usd.saturating_add(cost_micro_usd);
        // The normalized usage log (#1567) rides every production usage path,
        // not only the legacy whole-result accumulator.
        if input_tokens > 0
            || output_tokens > 0
            || cache_read_tokens > 0
            || cache_write_tokens > 0
            || cost_micro_usd > 0
        {
            let ratio = self.usage.cache_hit_ratio();
            tracing::info!(
                target: "session_usage",
                session_key = %session_key,
                input = self.usage.tokens.input,
                output = self.usage.tokens.output,
                cacheRead = self.usage.tokens.cache_read,
                cacheWrite = self.usage.tokens.cache_write,
                total = self.usage.tokens.total,
                contextTokens = self.context_tokens,
                costMicroUsd = self.usage.cost_micro_usd,
                cacheHitRatio = ratio,
                "normalized session usage recorded"
            );
        }
    }
    pub(crate) fn record_request_diagnostics(
        &mut self,
        diagnostics: crate::domain::request_observation::RequestDiagnostics,
    ) {
        self.usage.request_diagnostics.merge(diagnostics);
        self.bump_visible_generation();
    }

    pub fn usage_snapshot(&self) -> SessionUsage {
        self.usage.clone()
    }
    pub fn clear_usage(&mut self) {
        self.usage = SessionUsage::default();
        self.context_tokens = 0;
    }
    /// Maximum number of pending (steer/follow_up) messages buffered at once.
    /// Prevents OOM from a flood of pending messages from a misbehaving client.
    pub const MAX_PENDING: usize = 64;
    pub(crate) const MAX_DEDUPE_AGENTS: usize = 1024;
    pub fn enqueue_pending(&mut self, msg: String) -> bool {
        if self.pending.len() >= Self::MAX_PENDING {
            return false;
        }
        self.pending.push_back(PendingMessage::user(msg));
        true
    }
    /// Retain a steering message before earlier follow-ups, if capacity permits.
    pub fn prepend_pending(&mut self, msg: String) -> bool {
        if self.pending.len() >= Self::MAX_PENDING {
            return false;
        }
        self.pending.push_front(PendingMessage::user(msg));
        true
    }
    /// Test-only: simulate dedupe-watermark eviction at the
    /// `MAX_DEDUPE_AGENTS` cap (#1082 review round 2).
    #[cfg(test)]
    pub fn clear_subagent_notification_watermarks_for_test(&mut self) {
        self.last_subagent_notification.clear();
    }
    /// Return already-admitted work after steering interrupts a pending batch.
    pub fn restore_pending(&mut self, messages: impl DoubleEndedIterator<Item = PendingMessage>) {
        for message in messages.rev() {
            self.pending.push_front(message);
        }
    }

    pub(crate) fn discard_pending(&mut self) {
        for message in self.drain_pending() {
            if let PendingMessage::Control { id, command, .. } = message {
                self.record_control(
                    Some(&id),
                    &command,
                    super::protocol::ControlStatus::Cancelled,
                );
            }
        }
    }

    pub fn drain_pending(&mut self) -> Vec<PendingMessage> {
        // Vec::from(VecDeque) calls make_contiguous() then ptr::copy when the
        // deque's head != 0 — O(n) in the number of elements, same as the
        // previous .into_iter().collect().  Pending queue is capped at 64
        // entries so worst case is ~64 fat-pointer copies (~1.5 KiB).
        let mut drained = Vec::from(std::mem::take(&mut self.pending));
        // #1082 review round 2: notes retained under a full queue drain here
        // too, so queue saturation delays but never loses them.
        drained.extend(std::mem::take(&mut self.overflow_notifications));
        drained
    }
    /// `effort` is the agent loop's effective level plus the active model's
    /// catalogue vocabulary (#1067, #1848); both live outside this tracker
    /// (the agent and the change-reasoning-effort use case), so callers pass
    /// the presented view in and every `get_state` shape (live or snapshot)
    /// carries it. `session_key` is the active session's key (D10 #1979):
    /// the tracker holds no copy, avoiding stale per-session state.
    pub fn state_snapshot(
        &self,
        session_key: &str,
        message_count: usize,
        workflow: Option<serde_json::Value>,
        max_context_tokens: usize,
        effort: impl Into<crate::interface::uds::catalogue::effort_presenter::EffortStateView>,
    ) -> SessionState {
        let effort = effort.into();
        SessionState {
            admission_warnings: self.current_admission_warnings(),
            control_receipts: self.control_receipts.clone(),
            model: self.model.clone(),
            generation: self.generation,
            is_streaming: self.streaming,
            session_key: session_key.to_owned(),
            message_count,
            pending_message_count: self.pending.len(),
            max_context_tokens,
            effort: effort.effort,
            effort_levels: effort.effort_levels,
            workflow,
            execution: None,
            sync: 1,
            automatic_turns_suspended: !self.automatic_turns_allowed,
            repeated_failure_notifications: self.repeated_failure_notifications,
        }
    }
}
// ─── Session statistics ───────────────────────────────────────────────────────
#[path = "uds_session_usage.rs"]
mod usage_projection;
pub use usage_projection::{SessionUsage, compute_session_stats, compute_session_stats_with_usage};
#[path = "uds_session_history.rs"]
pub(crate) mod uds_session_history;
#[cfg(test)]
pub(crate) use uds_session_history::messages_page_json;
pub(crate) use uds_session_history::{
    HISTORY_PAGE_JSON_BUDGET, HISTORY_PAGE_SIZE, history_page_json,
    message_to_json_for_history_page,
};

/// Static wire name for a role — no per-message throwaway `String` allocation
/// (previously `format!("{:?}", role).to_lowercase()`, two heap allocs) (#994).
pub(crate) fn role_wire_name(role: &Role) -> &'static str {
    role.as_str()
}
/// A borrowed, zero-copy `Serialize` view of a [`Message`] in the UDS protocol
/// shape. Serializes straight from the typed message into the output (writer,
/// string, or `serde_json::Value`) without building an intermediate `json!`
/// tree or allocating a role string per message (#994).
pub(crate) struct MessageView<'a>(pub &'a Message);
#[derive(serde::Serialize)]
struct ToolCallView<'a> {
    id: &'a str,
    name: &'a str,
    arguments: &'a str,
}
struct ToolCallsView<'a>(&'a [crate::domain::message::ToolCall]);
impl serde::Serialize for ToolCallsView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for tc in self.0 {
            seq.serialize_element(&ToolCallView {
                id: &tc.id,
                name: &tc.name,
                arguments: &tc.arguments,
            })?;
        }
        seq.end()
    }
}
impl serde::Serialize for MessageView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let msg = self.0;
        // 8 base fields: stable id (#1060) + role/content/tools + isError + collapsed
        // (a demoted stub the client recalls by id; #1061 / ADR-0008 part 3).
        // Assistant thinking is an additive, display-safe recovery field (#1231).
        let field_count = if msg.thinking_blocks.is_empty() {
            9
        } else {
            10
        };
        let mut s = serializer.serialize_struct("Message", field_count)?;
        // Domain UUID as a round-trippable string key (AC6).
        s.serialize_field("id", &msg.id().to_string())?;
        s.serialize_field("ordinal", &msg.ordinal)?;
        s.serialize_field("role", role_wire_name(&msg.role))?;
        s.serialize_field("content", &msg.content)?;
        s.serialize_field("toolCalls", &ToolCallsView(&msg.tool_calls))?;
        s.serialize_field("toolCallId", &msg.tool_call_id)?;
        s.serialize_field("toolName", &msg.tool_name)?;
        s.serialize_field("isError", &msg.is_error)?;
        // Ladder-collapsed stub: rendered in place, full body recallable by id.
        s.serialize_field("collapsed", &msg.is_collapsed)?;
        if !msg.thinking_blocks.is_empty() {
            s.serialize_field(
                "thinking",
                &uds_visible_thinking_wire::visible_thinking_blocks_json(&msg.thinking_blocks),
            )?;
        }
        s.end()
    }
}
/// Serialize a `Message` to a JSON value for protocol emission.
///
/// Prefer serializing [`MessageView`] directly to a writer/string where a
/// `Value` is not actually needed (see `build_get_messages_line`); this helper
/// exists for the query paths whose event `data` field is a `serde_json::Value`.
pub fn message_to_json(msg: &Message) -> serde_json::Value {
    serde_json::to_value(MessageView(msg)).unwrap_or_default()
}
#[path = "uds_session_message_range.rs"]
mod uds_session_message_range;
#[path = "uds_visible_thinking_wire.rs"]
mod uds_visible_thinking_wire;
pub(crate) use uds_session_message_range::recovered_content_json;
#[cfg(test)]
#[path = "uds_session_coalesce_tests.rs"]
mod coalesce_pending_tests;
#[cfg(test)]
#[path = "uds_session_failure_tests.rs"]
mod failure_tests;
#[cfg(test)]
#[path = "uds_session_notification_tests.rs"]
mod notification_tests;
#[cfg(test)]
#[path = "uds_session_1060_tests.rs"]
mod uds_session_1060_tests;

#[path = "uds_session_controls.rs"]
mod controls;

#[cfg(test)]
#[path = "uds_session_suspension_tests.rs"]
mod suspension_tests;
