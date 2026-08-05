use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::domain::ids::AgentUuid;
use crate::domain::subagent::{DisplayNameResolutionEntry, resolve_live_display_name};

use super::subagent_lifecycle::{SubagentLifecycleEvent, SubagentLifecycleState};
pub use super::subagent_status::SubagentStatus;

/// Entry for a spawned subagent in the shared registry.
#[derive(Debug, Clone)]
pub struct SubagentEntry {
    /// Hidden durable identity for this spawn. Registry/storage/socket owners use
    /// this value as the stable agent identity; display labels remain wire/UI names.
    pub agent_uuid: AgentUuid,
    /// User-facing display label (`agent_id` on the compatibility wire).
    pub display_name: String,
    /// Path to the child's UDS socket.
    pub socket_path: PathBuf,
    /// Child process PID (0 in stub mode).
    pub pid: u32,
    /// Explicit internal lifecycle state. Parent-facing status is projected from
    /// this richer state so lifecycle races can be tested without changing the
    /// existing UDS status vocabulary.
    pub lifecycle: SubagentLifecycleState,
    /// Live status updated by the monitor task (#522).
    pub status: SubagentStatus,
    /// Name of the last tool being executed (from tool_execution_start).
    pub last_tool: Option<String>,
    /// Description of the last terminal/run-level error (for example agent_error).
    pub last_error: Option<String>,
    /// Run-level agent error (for example provider/model failure). Unlike a tool
    /// error, this means the prompt run failed and `agent_cmd await` should
    /// return a structured error instead of waiting for recovery.
    pub run_error: Option<String>,
    /// When this entry was last updated by the monitor.
    pub updated_at: Instant,
    /// Abort handle for the monitor task (if running).
    pub monitor_handle: Option<Arc<tokio::task::JoinHandle<()>>>,
    /// Monotonic notification id for this subagent.
    pub notification_sequence: u64,
    /// Exit signal sender — the reaper task sends the exit code/signal through
    /// this channel so that a waiting `await` call can return immediately (#612).
    pub exit_signal_tx: Option<ExitSignalTx>,
    /// The spawning agent's id, for reconstructing the unit tree (PRD Stage B).
    pub parent_id: Option<String>,
    /// Latest workflow snapshot reported by the child's monitor (PRD Stage B).
    pub workflow: Option<WorkflowSnapshot>,
    /// Set by `execute_await`'s TERMINAL result, consumed by the dispatch loop to
    /// suppress the duplicate passive note, re-armed on a new run (#await-dedupe).
    pub completion_consumed_by_await: bool,
    /// Terminal-completion latch (#904): consumed by the first `complete`-mode
    /// `agent_end`, re-armed when the workflow leaves `complete`.
    pub completion_armed: bool,
    /// One-shot stalled-notification latch (#1076): consumed when a non-terminal
    /// workflow stall is reported, re-armed by a new run or workflow progress.
    pub stalled_armed: bool,
    /// A supervision-critical stall alert retained after notification-channel
    /// saturation and retried on the next monitor event (#1076).
    pub pending_stall: Option<SequencedSubagentNotification>,
    pub read_only: bool,
    pub cleanup_environment_id: Option<String>,
    pub cleanup_argv: Vec<String>,
    /// Session environment-registry handle plus the minted `CN` ref, taken
    /// together with the cleanup plan so the environment entry is uncommitted
    /// exactly once when this agent's cleanup runs.
    pub environment_registry: Option<crate::domain::environment_registry::EnvironmentRegistry>,
    pub environment_ref: Option<String>,
    /// Last lifecycle event applied to this entry. This is internal observability
    /// for race-focused tests; parent-facing behavior continues to use `status`.
    #[cfg(test)]
    pub last_lifecycle_event: Option<SubagentLifecycleEvent>,
}

pub(super) fn seed_bound_workflow(
    entry: &mut SubagentEntry,
    workflow_spec: Option<&crate::domain::workflow::WorkflowSpec>,
) {
    let Some(spec) = workflow_spec else { return };
    entry.workflow = Some(WorkflowSnapshot {
        mode: crate::domain::workflow::WorkflowMode::Active
            .wire_str()
            .to_string(),
        steps_completed: 0,
        steps_total: u32::try_from(spec.template.steps.len()).unwrap_or(u32::MAX),
    });
}

impl SubagentEntry {
    /// Display label to expose for this entry. Legacy tests and hand-built
    /// registries may still be keyed by display name; UUID-keyed production
    /// entries carry an explicit display_name.
    pub fn effective_display_name<'a>(&'a self, registry_key: &'a str) -> &'a str {
        if self.display_name.is_empty() || registry_key != self.agent_uuid.as_str() {
            registry_key
        } else {
            self.display_name.as_str()
        }
    }

    /// Create a new entry with `Starting` status.
    pub fn new(socket_path: PathBuf, pid: u32) -> Self {
        let display_name = socket_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        Self::with_identity(AgentUuid::mint(), display_name, socket_path, pid)
    }

    /// Create a new entry with explicit hidden identity and display label.
    pub fn with_identity(
        agent_uuid: AgentUuid,
        display_name: String,
        socket_path: PathBuf,
        pid: u32,
    ) -> Self {
        Self {
            agent_uuid,
            display_name,
            socket_path,
            pid,
            lifecycle: SubagentLifecycleState::Launched,
            status: SubagentStatus::Starting,
            last_tool: None,
            last_error: None,
            run_error: None,
            updated_at: Instant::now(),
            monitor_handle: None,
            notification_sequence: 0,
            exit_signal_tx: None,
            parent_id: None,
            workflow: None,
            completion_consumed_by_await: false,
            completion_armed: true,
            stalled_armed: true,
            pending_stall: None,
            read_only: false,
            cleanup_environment_id: None,
            cleanup_argv: Vec::new(),
            environment_registry: None,
            environment_ref: None,
            #[cfg(test)]
            last_lifecycle_event: None,
        }
    }
}

/// Resolve a caller-supplied agent reference (live display label or UUID) to
/// the durable registry key. Prefer exact UUID key hits (including exited
/// entries), then fall back to live-only display-name resolution (#1378).
///
/// Live-only display resolution keeps dead agents non-targetable for NEW
/// commands (socket lookup / await arming). Await-dedupe uses
/// [`resolve_registry_key_for_await_dedupe`] so an exit note that still shows
/// the display label can coalesce against a retained Exited entry.
pub fn resolve_registry_key(
    entries: &HashMap<String, SubagentEntry>,
    agent_ref: &str,
) -> Result<String, crate::domain::subagent::DisplayNameResolveError> {
    if entries.contains_key(agent_ref) {
        return Ok(agent_ref.to_string());
    }
    let resolution_entries = display_resolution_entries(entries);
    resolve_live_display_name(&resolution_entries, agent_ref).map(|uuid| uuid.into_string())
}

/// Like [`resolve_registry_key`], but display-label fallback also matches a
/// unique retained **exited** entry. Used only for await-dedupe coalescing so
/// notes that embed the user-facing label still suppress after `mark_exited`
/// (#1378 adversarial re-review). Does not re-arm sockets or resume sessions.
pub fn resolve_registry_key_for_await_dedupe(
    entries: &HashMap<String, SubagentEntry>,
    agent_ref: &str,
) -> Result<String, crate::domain::subagent::DisplayNameResolveError> {
    if entries.contains_key(agent_ref) {
        return Ok(agent_ref.to_string());
    }
    if let Some((key, _)) = entries.iter().find(|(key, entry)| {
        entry.effective_display_name(key) == agent_ref && entry.completion_consumed_by_await
    }) {
        return Ok(key.clone());
    }

    let resolution_entries = display_resolution_entries(entries);
    match resolve_live_display_name(&resolution_entries, agent_ref) {
        Ok(uuid) => Ok(uuid.into_string()),
        Err(crate::domain::subagent::DisplayNameResolveError::NoLiveMatch { .. }) => {
            resolve_unique_retained_display_name(&resolution_entries, agent_ref)
                .map(|uuid| uuid.into_string())
        }
        Err(err) => Err(err),
    }
}

fn display_resolution_entries(
    entries: &HashMap<String, SubagentEntry>,
) -> Vec<DisplayNameResolutionEntry> {
    entries
        .iter()
        .map(|(key, entry)| DisplayNameResolutionEntry {
            agent_uuid: entry.agent_uuid.clone(),
            display_name: entry.effective_display_name(key).to_string(),
            live: entry.status != SubagentStatus::Exited,
        })
        .collect()
}

/// Unique retained display-name match across live **and** exited entries.
/// Prefers not to invent multi-match semantics: zero → NoLiveMatch, 2+ →
/// AmbiguousLiveMatch (same error vocabulary as live resolve).
fn resolve_unique_retained_display_name(
    entries: &[DisplayNameResolutionEntry],
    display_name: &str,
) -> Result<crate::domain::ids::AgentUuid, crate::domain::subagent::DisplayNameResolveError> {
    let mut matches = entries
        .iter()
        .filter(|entry| entry.display_name == display_name)
        .map(|entry| entry.agent_uuid.clone());

    let Some(first) = matches.next() else {
        return Err(
            crate::domain::subagent::DisplayNameResolveError::NoLiveMatch {
                display_name: display_name.to_string(),
            },
        );
    };
    if matches.next().is_some() {
        return Err(
            crate::domain::subagent::DisplayNameResolveError::AmbiguousLiveMatch {
                display_name: display_name.to_string(),
            },
        );
    }
    Ok(first)
}

/// Mark `agent_ref`'s entry as having had its current-run terminal completion
/// consumed by a manual `await` (auto-await dedupe). `agent_ref` may be a live
/// display label, retained exited display label, or UUID; the flag is always
/// stored on the UUID-keyed entry (#1378). No-op if the entry no longer exists.
pub fn mark_completion_consumed_by_await(registry: &SubagentRegistry, agent_ref: &str) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(key) = resolve_registry_key_for_await_dedupe(&entries, agent_ref) else {
        return;
    };
    if let Some(entry) = entries.get_mut(&key) {
        entry.completion_consumed_by_await = true;
        entry.status = super::subagent_lifecycle::apply_lifecycle_event(
            &mut entry.lifecycle,
            SubagentLifecycleEvent::AwaitConsumedCompletion,
        );
    }
}

/// Check-and-consume the await-dedupe flag for `agent_ref`. Returns `true` when
/// the passive completion note should be SUPPRESSED because a manual `await`
/// already reported this terminal result; in that case the flag is cleared so a
/// future re-run still notifies. Returns `false` otherwise. Race-free against
/// `execute_await`: the UDS dispatch loop is single-threaded, so the await tool
/// call sets the flag before the loop processes the queued notification.
/// `agent_ref` may be a live / retained-exited display label or UUID (#1378).
pub fn take_completion_consumed_by_await(registry: &SubagentRegistry, agent_ref: &str) -> bool {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(key) = resolve_registry_key_for_await_dedupe(&entries, agent_ref) else {
        return false;
    };
    if let Some(entry) = entries.get_mut(&key) {
        if entry.completion_consumed_by_await {
            entry.completion_consumed_by_await = false;
            entry.status = super::subagent_lifecycle::apply_lifecycle_event(
                &mut entry.lifecycle,
                SubagentLifecycleEvent::PassiveNoteEmitted,
            );
            return true;
        }
    }
    false
}

/// Check-and-consume the await-dedupe flag for `agent_id` against an OPTIONAL
/// registry, the form both UDS dispatch paths hold (#828). Returns `true` when
/// the passive completion note should be suppressed (a manual `await` already
/// reported it); `false` when there is no registry or no pending flag. Wraps
/// [`take_completion_consumed_by_await`] so the predicate lives in ONE place.
pub fn consume_await_dedupe(registry: &Option<SubagentRegistry>, agent_id: &str) -> bool {
    registry
        .as_ref()
        .is_some_and(|reg| take_completion_consumed_by_await(reg, agent_id))
}

pub type SubagentRegistry = Arc<Mutex<HashMap<String, SubagentEntry>>>;

pub fn new_registry() -> SubagentRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Maximum wall-clock time to wait for a forwarded sub-agent UDS response on the
/// `agent_cmd` path (a tool call that may legitimately wait on a long operation).
const SUBAGENT_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Short, interactive-scale timeout for forwards driven by the TUI inspector's
/// 1s poll loop (#795). A `get_messages_tail` query answers from history almost
/// instantly; capping at a few seconds keeps a slow/hung sub-agent from
/// head-of-line-blocking the parent's shared dispatch loop (review: DoS / perf).
pub const INSPECTOR_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Per-line cap on a sub-agent's UDS reply (#795 security review). Mirrors the
/// inbound client cap (`uds::MAX_FRAME_PAYLOAD_BYTES`) so a misbehaving/compromised
/// sub-agent cannot return an unbounded line and exhaust the parent's memory.
const SUBAGENT_RESPONSE_MAX_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;

/// Look up the UDS socket path for a spawned sub-agent by id.
///
/// Single source of truth for both `agent_cmd` forwarding and the TUI's
/// agent-targeted `get_messages_tail` so the lookup rule never diverges (#795).
pub fn lookup_subagent_socket(
    registry: &SubagentRegistry,
    agent_id: &str,
) -> Result<PathBuf, String> {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let key = resolve_registry_key(&entries, agent_id).map_err(|err| match err {
        crate::domain::subagent::DisplayNameResolveError::NoLiveMatch { display_name } => {
            format!("no live subagent named '{display_name}' (not found)")
        }
        crate::domain::subagent::DisplayNameResolveError::AmbiguousLiveMatch { display_name } => {
            format!("duplicate live subagent display label '{display_name}'")
        }
    })?;
    entries
        .get(&key)
        .map(|e| e.socket_path.clone())
        .ok_or_else(|| format!("subagent '{}' not found in registry", agent_id))
}

/// Send a framed JSON command to a sub-agent's UDS socket and read back the
/// `response` message that matches the command we sent.
///
/// Each call opens a new connection, stamps a unique `id` on the command, writes
/// one frame, and reads messages until the `{"type":"response","id":<that id>,...}`
/// event arrives (skipping tokens, agent_start, and unsolicited responses such as
/// the connect-time `get_messages` snapshot — built with no id — that the
/// broadcast delivers to all clients).
/// Shared by
/// `agent_cmd` and the TUI agent-targeted tail forwarder so the framing rule
/// lives in one place (#795).
///
/// Uses the long [`SUBAGENT_RESPONSE_TIMEOUT`]; interactive callers that must
/// not block the shared dispatch loop should use
/// [`send_subagent_uds_command_with_timeout`] with [`INSPECTOR_RESPONSE_TIMEOUT`].
pub async fn send_subagent_uds_command(
    socket_path: &std::path::Path,
    command: &str,
) -> Result<String, crate::domain::error::DomainError> {
    send_subagent_uds_command_with_timeout(socket_path, command, SUBAGENT_RESPONSE_TIMEOUT).await
}

/// Like [`send_subagent_uds_command`] but with an explicit response timeout, so
/// interactive callers (the TUI inspector poll, #795) can cap head-of-line
/// blocking on the parent's shared command loop.
pub async fn send_subagent_uds_command_with_timeout(
    socket_path: &std::path::Path,
    command: &str,
    response_timeout: std::time::Duration,
) -> Result<String, crate::domain::error::DomainError> {
    use crate::domain::error::DomainError;
    use tokio::io::BufReader;

    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .map_err(|e| {
            DomainError::Tool(format!(
                "connect to subagent at {} failed: {e}",
                socket_path.display()
            ))
        })?;

    let (reader, mut writer) = tokio::io::split(stream);

    // Correlate the reply with the command we SENT via a unique request `id`
    // (the protocol's correlation field): we stamp a fresh id and accept the
    // `response` whose `id` echoes it (`AgentEvent::ok(id, ..)`). Unsolicited
    // responses — notably the connect-time `get_messages` SNAPSHOT a BUSY child
    // pushes on every new connection (#828, `id: None`) — carry no id and are
    // skipped here, so a parent no longer consumes that snapshot's FIRST message
    // instead of the real reply (#831). id-matching also disambiguates two
    // responses sharing a command (a `get_messages` request vs. the snapshot).
    // A non-object command can't be stamped, so we fall back to first-response.
    let (outbound, expected_id) = stamp_request_id(command);

    // Parent and child are the same binary, so outbound commands always use
    // ADR-0008 framing. Compatibility remains reader-side and is sniffed for
    // every incoming message; it does not pin or downgrade this writer.
    quecto_line_io::write_frame(
        &mut writer,
        outbound.as_bytes(),
        quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
    )
    .await
    .map_err(|e| DomainError::Tool(format!("write to subagent failed: {e}")))?;
    // Do NOT shutdown or drop the write half (#557): in multi-client mode the
    // server's reader loop exits on EOF and aborts the broadcast writer, so the
    // response would never arrive. Keep the write half alive until we're done.
    let _keep_alive = writer;

    let mut reader = BufReader::new(reader);
    let deadline = tokio::time::Instant::now() + response_timeout;
    let timeout_msg = || {
        format!(
            "subagent response timed out ({}s)",
            response_timeout.as_secs()
        )
    };
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(DomainError::Tool(timeout_msg()));
        }
        let line = tokio::time::timeout(
            remaining,
            read_response_capped(&mut reader, SUBAGENT_RESPONSE_MAX_BYTES),
        )
        .await
        .map_err(|_| DomainError::Tool(timeout_msg()))??;
        match line {
            Some(l) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&l) {
                    if json.get("type").and_then(|v| v.as_str()) == Some("response") {
                        match &expected_id {
                            // We stamped an id on the request: only accept the
                            // response that echoes it, skipping unsolicited
                            // responses (the connect-time snapshot, which carries
                            // no id, and any other interleaved reply).
                            Some(expected) => {
                                if json.get("id").and_then(|v| v.as_str()) == Some(expected) {
                                    return Ok(l);
                                }
                                if subagent_snapshot::response_is_valid_answer(&json, command) {
                                    // Accept the id-less snapshot, applying the
                                    // request's `count` locally (last-N tail, #842).
                                    return Ok(subagent_snapshot::finalize_snapshot_answer(
                                        l, json, command,
                                    ));
                                }
                                // Unsolicited / mismatched response — skip.
                            }
                            // Command wasn't a JSON object we could stamp: fall
                            // back to historical behaviour (first response).
                            None => return Ok(l),
                        }
                    }
                }
                // Not a response event — skip.
            }
            None => {
                return Err(DomainError::Tool(
                    "subagent closed connection without sending a response".into(),
                ));
            }
        }
    }
}

/// Stamp a unique correlation `id` onto an outbound UDS command so the read loop
/// can match the reply by its echoed `id` field, skipping unsolicited responses
/// such as the connect-time `get_messages` snapshot (which carries no id) (#831).
///
/// Returns the (possibly rewritten) command line to send and the id to match on.
/// When the command is not a JSON object we cannot stamp an id, so the original
/// command is returned with `None`, signalling the caller to fall back to
/// first-response behaviour. Any pre-existing `id` is overwritten so two callers
/// reusing the same literal command can never collide.
fn stamp_request_id(command: &str) -> (String, Option<String>) {
    match serde_json::from_str::<serde_json::Value>(command) {
        Ok(serde_json::Value::Object(mut map)) => {
            let id = uuid::Uuid::new_v4().to_string();
            map.insert("id".to_owned(), serde_json::Value::String(id.clone()));
            (serde_json::Value::Object(map).to_string(), Some(id))
        }
        _ => (command.to_owned(), None),
    }
}

/// Read the next sub-agent message, capping (rather than buffering) any message
/// that exceeds `max_bytes` (#795 security review) so a sub-agent cannot OOM the
/// parent with one giant message, while still allowing an unbounded number of
/// normal-sized messages to be skipped before the `response` event arrives.
///
/// Delegates the sniff-and-cap framing to the shared
/// [`quecto_line_io::read_frame_or_legacy_line`] helper (#1059) so this
/// parent→child consumer shares the same length-prefixed-frame / legacy-NDJSON
/// deprecation-window handling as the other four UDS consumers; over-cap
/// messages are skipped (not hard-errored) here.
async fn read_response_capped<R>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<String>, crate::domain::error::DomainError>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use crate::domain::error::DomainError;
    use quecto_line_io::{FrameError, Incoming};

    // Deprecation-window reader (#1059): each reply is sniffed as a
    // length-prefixed frame or a legacy NDJSON line. An over-cap interleaved
    // message is skipped (the declared/until-newline bytes were consumed, so
    // the stream stays framed) rather than hard-erroring the whole query — the
    // same skip-and-continue the other four consumers use, replacing the old
    // "response line exceeded size limit" abort. An unknown first byte is an
    // explicit `VersionMismatch`, never a silent misparse.
    loop {
        match quecto_line_io::read_frame_or_legacy_line(reader, max_bytes).await {
            Ok(None) => return Ok(None),
            Ok(Some(incoming)) => {
                let bytes = match incoming {
                    Incoming::Frame(b) | Incoming::LegacyLine(b) => b,
                };
                return Ok(Some(String::from_utf8(bytes).unwrap_or_else(|e| {
                    String::from_utf8_lossy(e.as_bytes()).into_owned()
                })));
            }
            Err(FrameError::Oversized { .. }) => continue,
            Err(e) => {
                return Err(DomainError::Tool(format!("read from subagent failed: {e}")));
            }
        }
    }
}

// ─── Await support (#612) ────────────────────────────────────────────────────
// `AwaitResult`, `WorkflowSnapshot`, `WorkflowResult`, `ResultProgress` and the
// verdict-derivation logic live in the `subagent_await_result` child module
// (declared near the bottom of this file) to respect the 750-line file cap; they
// are re-exported below so `subagent_registry::AwaitResult` etc. keep working.
/// Signal sent by the reaper task when a child process exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitSignal {
    /// Process exit code (0 for success, non-zero for error).
    /// `None` if the process was killed by a signal.
    pub exit_code: Option<i32>,
    /// Signal number if the process was killed by a signal.
    pub signal: Option<i32>,
}

pub type ExitSignalTx = tokio::sync::watch::Sender<Option<ExitSignal>>;
pub type ExitSignalRx = tokio::sync::watch::Receiver<Option<ExitSignal>>;

pub fn new_exit_signal_channel() -> (ExitSignalTx, ExitSignalRx) {
    tokio::sync::watch::channel(None)
}

/// Tracks which agent_ids have an active `await` call to prevent duplicates.
pub type ActiveAwaits = Arc<Mutex<HashSet<String>>>;

/// Create a new empty active awaits tracker.
pub fn new_active_awaits() -> ActiveAwaits {
    Arc::new(Mutex::new(HashSet::new()))
}

// ─── Subagent notifications (#523) ───────────────────────────────────────────

/// A notification from a child agent to the parent dispatch loop (#523).
///
/// Sent by the monitor task when a child reaches a terminal or notable state.
/// The parent dispatch loop injects these as follow-up messages to the LLM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentNotification {
    /// Child agent ended a turn and was observed idle; this is not a task-success verdict.
    Completed { agent_id: String },
    /// Workflow-bound child became idle before reaching a terminal workflow state.
    Stalled {
        agent_id: String,
        workflow_mode: String,
        steps_completed: u64,
        steps_total: u64,
    },
    /// Child agent's last tool execution returned an error.
    Errored { agent_id: String, error: String },
    /// Child agent process exited (connection closed or process reaped).
    Exited { agent_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequencedSubagentNotification {
    pub sequence: u64,
    pub notification: SubagentNotification,
    /// Hidden generation identity for internal routing (#1378).
    pub agent_uuid: Option<AgentUuid>,
}

impl SequencedSubagentNotification {
    pub fn new(sequence: u64, notification: SubagentNotification) -> Self {
        Self {
            sequence,
            notification,
            agent_uuid: None,
        }
    }

    pub fn new_for_agent(
        sequence: u64,
        notification: SubagentNotification,
        agent_uuid: AgentUuid,
    ) -> Self {
        Self {
            sequence,
            notification,
            agent_uuid: Some(agent_uuid),
        }
    }

    pub fn dedupe_key(&self) -> (String, u64) {
        let agent_id = match &self.notification {
            SubagentNotification::Completed { agent_id, .. }
            | SubagentNotification::Stalled { agent_id, .. }
            | SubagentNotification::Errored { agent_id, .. }
            | SubagentNotification::Exited { agent_id } => agent_id.clone(),
        };
        (agent_id, self.sequence)
    }

    /// Internal await-dedupe reference: UUID when stamped, else display label.
    pub fn await_dedupe_key(&self) -> (String, u64) {
        self.agent_uuid
            .as_ref()
            .map(|uuid| (uuid.to_string(), self.sequence))
            .unwrap_or_else(|| self.dedupe_key())
    }

    pub fn to_message(&self) -> String {
        self.notification.to_message()
    }

    /// `true` only for normal idle turn ends; failures must not coalesce (#894).
    pub fn is_completion(&self) -> bool {
        matches!(self.notification, SubagentNotification::Completed { .. })
    }
}

impl SubagentNotification {
    /// Format this notification as a human-readable parent message.
    pub fn to_message(&self) -> String {
        // One line; soft, not imperative (#894); #926-AC2 actionability deferred.
        match self {
            Self::Completed { agent_id, .. } => format!(
                "Sub-agent '{agent_id}' ended a turn (status: idle). Inspect agent_cmd get_messages before treating its work as complete."
            ),
            Self::Stalled {
                agent_id,
                workflow_mode,
                steps_completed,
                steps_total,
            } => format!(
                "Agent '{agent_id}' stalled: idle with workflow still {workflow_mode} at {steps_completed}/{steps_total}. Inspect output/state, then prompt, steer, abort, or kill it."
            ),
            Self::Errored { agent_id, error } => format!("Agent '{agent_id}' failed: {error}"),
            Self::Exited { agent_id } => format!("Agent '{agent_id}' exited unexpectedly"),
        }
    }
}

/// Sender half of the notification channel.
pub type NotificationTx = tokio::sync::mpsc::Sender<SequencedSubagentNotification>;

/// Receiver half of the notification channel.
pub type NotificationRx = tokio::sync::mpsc::Receiver<SequencedSubagentNotification>;

/// Default capacity for the bounded notification channel.
pub const NOTIFICATION_CHANNEL_CAPACITY: usize = 64;

/// Create a new bounded notification channel.
pub fn new_notification_channel() -> (NotificationTx, NotificationRx) {
    tokio::sync::mpsc::channel(NOTIFICATION_CHANNEL_CAPACITY)
}

/// Validate an agent_id string for format (shared between spawn and agent_cmd).
pub fn validate_agent_id_format(agent_id: &str) -> Result<(), String> {
    if agent_id.is_empty() || agent_id.len() > 64 {
        return Err("agent_id must be 1-64 characters".to_string());
    }
    agent_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        .then_some(())
        .ok_or_else(|| "agent_id must use only [a-zA-Z0-9_-]".to_string())
}

#[path = "subagent_await_result.rs"]
mod subagent_await_result;
pub use subagent_await_result::{AwaitResult, ResultProgress, WorkflowResult, WorkflowSnapshot};

#[path = "subagent_snapshot.rs"]
mod subagent_snapshot;
#[cfg(test)]
#[path = "subagent_registry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "subagent_registry_cov_tests.rs"]
mod cov_tests;
