//! `agent_cmd` on a sub-agent that has ended (#2192). A live child answers
//! `get_state` and `get_messages` over its socket; an ended one cannot, so
//! the parent answers from what the child left: its end (the exit status
//! this harness observed and the crash record the child wrote) and its
//! persisted transcript. What cannot be read is said, with the reason.
use std::sync::{Arc, OnceLock};

use crate::application::subagents::use_cases::{
    EndedTranscriptError, EndedTranscriptPage, InspectEndedChild,
};
use crate::domain::child_end::{
    ChildEnd, ChildOrigin, MAX_SHOWN_NAME_BYTES, MAX_SHOWN_PANIC_BYTES, shown,
};
use crate::domain::ids::AgentUuid;
use crate::domain::message::Message;
use crate::domain::session::SubagentLiveness;
use crate::domain::tool::ToolResult;

use super::subagent_registry::{ExitSignalKind, SubagentRegistry};

/// Where composition installs the ended-child inspection, once, after the
/// agent-control tools are built.
#[derive(Clone, Default)]
pub struct EndedChildSlot(Arc<OnceLock<Arc<InspectEndedChild>>>);

impl EndedChildSlot {
    /// Install the inspection; `true` when this call filled the slot.
    pub fn install(&self, inspection: Arc<InspectEndedChild>) -> bool {
        self.0.set(inspection).is_ok()
    }

    pub fn get(&self) -> Option<Arc<InspectEndedChild>> {
        self.0.get().cloned()
    }
}

/// The messages a default `get_messages` of an ended child returns at most.
pub const DEFAULT_ENDED_MESSAGES: usize = 40;
/// The most messages one `get_messages` page of an ended child asks for.
pub const MAX_ENDED_PAGE: usize = InspectEndedChild::MAX_PAGE_MESSAGES;
/// The most transcript text one answer carries, in bytes (#2114's report
/// budget): older messages beyond it are left for a `before` page.
pub const ENDED_TRANSCRIPT_BUDGET: usize = 64 * 1024;
/// The most of one message's content an answer carries, in bytes.
const MESSAGE_CONTENT_BYTES: usize = 16 * 1024;
/// The most of one tool call's arguments an answer carries, in bytes.
const ARGUMENT_BYTES: usize = 1024;
/// The most of a tool call's id or name an answer carries, in bytes.
const NAME_BYTES: usize = 256;
/// The most tool calls of one message an answer carries.
const MAX_TOOL_CALLS: usize = 16;
/// The most of the child's last tool or error an answer shows, in bytes.
const LAST_ACTIVITY_BYTES: usize = 256;
/// What a believed crash record rests on, as the parent is told it.
pub const BELIEVED_BECAUSE: &str = "the exit status fits a fatal panic and the record names the \
     pid this harness launched the child with; the record itself is not authenticated";
/// How every child-supplied part of an answer is labelled.
const CHILD_SUPPLIED: &str = "child-supplied, unverified";

/// What the registry still knows of an ended child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedRow {
    /// Its label as shown: escaped and capped, since a merged descendant's
    /// comes from a child's own snapshot.
    pub label: String,
    pub uuid: AgentUuid,
    /// Its key in the registry: the row a default read's report is
    /// pending on and acknowledged for.
    pub key: String,
    pub last_tool: Option<String>,
    pub last_error: Option<String>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// The child's pid, when this harness launched it and knows it.
    pub pid: Option<u32>,
    /// Where the row came from: only a launched child's transcript is read.
    pub origin: ChildOrigin,
    /// Ended by this harness (a kill, a fleet teardown) or with an
    /// ancestor, rather than on its own.
    pub terminated: bool,
}

/// The ended row `reference` names: the row keyed by it, else the one row
/// with that uuid, else the one row with that display label. A row this
/// harness launched is looked for first (#2192 review): when one matches,
/// it is the answer — or, live or ambiguous, there is none — and a row a
/// child reported under the same name never stands in for it. `None` for
/// a live row or an unknown or ambiguous reference.
pub fn find_ended(registry: &SubagentRegistry, reference: &str) -> Option<EndedRow> {
    type Entry = super::subagent_registry::SubagentEntry;
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let ended = |entry: &Entry| entry.persisted_liveness == SubagentLiveness::Dead;
    let launched = |entry: &Entry| entry.origin == ChildOrigin::Launched;
    let names = |key: &str, entry: &Entry| {
        key == reference
            || entry.agent_uuid.as_str() == reference
            || entry.effective_display_name(key) == reference
    };
    let any_launched = entries
        .iter()
        .any(|(key, entry)| launched(entry) && names(key, entry));
    let found = match any_launched {
        true => lookup(&entries, reference, &launched).filter(|(_, entry)| ended(entry)),
        false => lookup(&entries, reference, &|entry: &Entry| ended(entry)),
    };
    let (key, entry) = found?;
    let exit = entry
        .exit_signal_tx
        .as_ref()
        .and_then(|tx| tx.borrow().clone());
    Some(EndedRow {
        label: shown(&entry.parent_facing_label(key), MAX_SHOWN_NAME_BYTES),
        uuid: entry.agent_uuid.clone(),
        key: key.clone(),
        last_tool: entry.last_tool.clone(),
        last_error: entry.last_error.clone(),
        exit_code: exit.as_ref().and_then(|exit| exit.exit_code),
        signal: exit.as_ref().and_then(|exit| exit.signal),
        pid: vouched_pid(entry),
        origin: entry.origin,
        terminated: exit
            .as_ref()
            .is_some_and(|exit| exit.kind == ExitSignalKind::Terminated),
    })
}

/// The pid of `entry`'s child, when this harness launched it and knows it
/// (#2192 review): any other row's pid is not this harness's to vouch for.
pub fn vouched_pid(entry: &super::subagent_registry::SubagentEntry) -> Option<u32> {
    match entry.origin {
        ChildOrigin::Launched => Some(entry.pid).filter(|pid| *pid > 0),
        ChildOrigin::Reported | ChildOrigin::Unverified => None,
    }
}

/// The end of `entry`'s child as its parent observed it: its exit status,
/// its pid when this harness launched it (the crash record must be that
/// process's), and the crash record it left.
pub fn child_end_of(
    entry: &super::subagent_registry::SubagentEntry,
    exit: Option<&super::subagent_registry::ExitSignal>,
    crash: Option<crate::domain::crash_record::CrashRecord>,
) -> ChildEnd {
    ChildEnd {
        exit_code: exit.and_then(|exit| exit.exit_code),
        signal: exit.and_then(|exit| exit.signal),
        pid: vouched_pid(entry),
        crash,
    }
}

/// The row among those `accept`ed that `reference` names: keyed by it,
/// else the one with that uuid, else the one with that display label.
fn lookup<'a>(
    entries: &'a std::collections::HashMap<String, super::subagent_registry::SubagentEntry>,
    reference: &str,
    accept: &dyn Fn(&super::subagent_registry::SubagentEntry) -> bool,
) -> Option<(&'a String, &'a super::subagent_registry::SubagentEntry)> {
    entries
        .get_key_value(reference)
        .filter(|(_, entry)| accept(entry))
        .or_else(|| {
            unique(
                entries
                    .iter()
                    .filter(|(_, e)| e.agent_uuid.as_str() == reference && accept(e))
                    .collect(),
            )
        })
        .or_else(|| {
            unique(
                entries
                    .iter()
                    .filter(|(key, e)| e.effective_display_name(key) == reference && accept(e))
                    .collect(),
            )
        })
}

/// The only item of `matches`, if there is exactly one.
fn unique<T>(mut matches: Vec<T>) -> Option<T> {
    match matches.len() {
        1 => matches.pop(),
        _ => None,
    }
}

/// Answer `command` from what an ended child left, when `agent_id` names
/// one (#2192); `None` leaves the caller's own refusal in place.
pub async fn answer_if_ended(
    (registry, inspection): (&SubagentRegistry, Option<Arc<InspectEndedChild>>),
    (agent_id, command, arguments): (&str, &str, &serde_json::Value),
) -> Option<ToolResult> {
    let row = find_ended(registry, agent_id)?;
    // The default read keeps the unread-report contract (#2192 review):
    // it needs the row's watermark, so it is answered here.
    match (report::is_default_read(command, arguments), &inspection) {
        (true, Some(inspection)) => {
            let reason = end_reason(&row, &row_end(inspection, &row).await);
            Some(report::default_read(registry, inspection, &row, &reason).await)
        }
        (true, None) | (false, _) => answer(inspection, &row, command, arguments).await,
    }
}

/// How `row`'s child ended, with the crash record it left read.
async fn row_end(inspection: &InspectEndedChild, row: &EndedRow) -> ChildEnd {
    ChildEnd {
        exit_code: row.exit_code,
        signal: row.signal,
        pid: row.pid,
        crash: inspection.crash(&row.uuid, row.origin, row.pid).await,
    }
}

/// Answer `command` for an ended child, or `None` when it is not one this
/// module answers (the caller keeps its own refusal).
pub async fn answer(
    inspection: Option<Arc<InspectEndedChild>>,
    row: &EndedRow,
    command: &str,
    arguments: &serde_json::Value,
) -> Option<ToolResult> {
    let mut end = ChildEnd {
        exit_code: row.exit_code,
        signal: row.signal,
        pid: row.pid,
        crash: None,
    };
    let Some(inspection) = inspection else {
        return Some(error(format!(
            "subagent '{}' {}; this harness cannot read what it left (no ended-child \
             inspection is composed)",
            row.label,
            end_reason(row, &end)
        )));
    };
    end.crash = inspection.crash(&row.uuid, row.origin, row.pid).await;
    let reason = end_reason(row, &end);
    match command {
        "get_state" => Some(state(row, &end, &reason)),
        "get_messages" => Some(messages(&inspection, row, &reason, arguments).await),
        "get_report" => Some(report::final_report(&inspection, row, &reason).await),
        _ => None,
    }
}

/// How the child ended, to follow its name: the same words everywhere the
/// end is shown (its exit note, `get_state`, `get_messages`).
pub fn end_reason(row: &EndedRow, end: &ChildEnd) -> String {
    match row.terminated {
        true => "was ended by this harness (or fell with an ancestor)".to_string(),
        false => end.reason(),
    }
}

fn state(row: &EndedRow, end: &ChildEnd, reason: &str) -> ToolResult {
    let data = serde_json::json!({
        "agentId": row.label,
        "agentUuid": row.uuid.as_str(),
        "status": "ended",
        "endReason": format!("subagent '{}' {reason}", row.label),
        "exitCode": end.exit_code,
        "signal": end.signal,
        "crash": crash_view(end),
        "lastActivity": {
            "provenance": CHILD_SUPPLIED,
            "tool": row.last_tool.as_deref().map(|tool| shown(tool, LAST_ACTIVITY_BYTES)),
            "error": row.last_error.as_deref().map(|error| shown(error, LAST_ACTIVITY_BYTES)),
        },
    });
    ok("get_state", data)
}

/// The crash record as the parent may see it: the child's words escaped,
/// capped and labelled, with whether they were believed.
fn crash_view(end: &ChildEnd) -> serde_json::Value {
    let Some(crash) = &end.crash else {
        return serde_json::Value::Null;
    };
    let name = |text: &str| shown(text, MAX_SHOWN_NAME_BYTES);
    serde_json::json!({
        "provenance": CHILD_SUPPLIED,
        "believed": end.believed_crash().is_some(),
        // What "believed" rests on (#2192 review): a match, not a proof.
        "believedBecause": BELIEVED_BECAUSE,
        "provisional": crash.provisional,
        "message": shown(&crash.panic.message, MAX_SHOWN_PANIC_BYTES),
        "location": crash.panic.location.as_deref().map(name),
        "call": crash.call.as_deref().map(name),
        "running": crash.running.iter().map(|tool| name(tool)).collect::<Vec<_>>(),
    })
}

/// The page size asked for: at least one message, [`DEFAULT_ENDED_MESSAGES`]
/// when none is named, and at most [`MAX_ENDED_PAGE`] — no more than one
/// answer's budget could carry (#2192 review: a huge `count` no longer
/// copies the whole transcript only to discard it).
fn page_size(arguments: &serde_json::Value) -> Result<usize, String> {
    match arguments.get("count") {
        None | Some(serde_json::Value::Null) => Ok(DEFAULT_ENDED_MESSAGES),
        Some(value) => match value.as_u64() {
            Some(count) if count > 0 => Ok(
                usize::try_from(count).map_or(MAX_ENDED_PAGE, |count| count.min(MAX_ENDED_PAGE))
            ),
            Some(_) | None => Err(format!(
                "'count' is a whole number of messages, at least 1: got {value}"
            )),
        },
    }
}

/// The cursor given: the ordinal an earlier answer named.
fn cursor(arguments: &serde_json::Value) -> Result<Option<u64>, String> {
    match arguments.get("before") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .map(Some)
            .ok_or_else(|| format!("'before' is the ordinal a previous answer gave: got {value}")),
    }
}

async fn messages(
    inspection: &InspectEndedChild,
    row: &EndedRow,
    reason: &str,
    arguments: &serde_json::Value,
) -> ToolResult {
    let (count, before) = match (page_size(arguments), cursor(arguments)) {
        (Ok(count), Ok(before)) => (count, before),
        (Err(why), _) | (_, Err(why)) => {
            return error(format!(
                "subagent '{}' has ended; its persisted transcript is paged by {why}",
                row.label
            ));
        }
    };
    match inspection
        .transcript(&row.uuid, row.origin, count, before)
        .await
    {
        Ok(page) => {
            let head = serde_json::json!({
                "ended": true,
                "endReason": format!("subagent '{}' {reason}", row.label),
                "source": "persisted transcript",
                "olderOmitted": page.older_omitted,
            });
            ok("get_messages", within_budget(head, &page))
        }
        Err(EndedTranscriptError::UnknownCursor(ordinal)) => error(format!(
            "subagent '{}' has ended; no message with ordinal {ordinal} is in its persisted \
             transcript",
            row.label
        )),
        Err(EndedTranscriptError::NotHere(why) | EndedTranscriptError::Unreadable(why)) => {
            error(format!(
                "subagent '{}' {reason}; its transcript isn't readable from here because {why}",
                row.label
            ))
        }
    }
}

/// The answer's data: `head`, then the newest messages of `page` that fit,
/// whole, in [`ENDED_TRANSCRIPT_BUDGET`] (the envelope counted), with the
/// cursor to the older ones. The newest message is always kept, cut down
/// until it fits. When older messages remain, the cursor names the oldest
/// kept one, so `before` is never null while `hasMoreBefore` is true.
fn within_budget(head: serde_json::Value, page: &EndedTranscriptPage) -> serde_json::Value {
    const CURSOR_SPACE: usize = 64;
    let envelope = |messages: &[serde_json::Value], before: Option<String>, more: bool| {
        let mut data = head.clone();
        data["messages"] = serde_json::Value::Array(messages.to_vec());
        data["before"] = serde_json::json!(before);
        data["hasMoreBefore"] = serde_json::json!(more);
        data
    };
    let room = ENDED_TRANSCRIPT_BUDGET
        .saturating_sub(ok("get_messages", envelope(&[], None, true)).content.len())
        .saturating_sub(CURSOR_SPACE);
    let mut kept: Vec<(serde_json::Value, Option<u64>)> = Vec::new();
    let mut used = 0usize;
    for (newest, message) in page.messages.iter().rev().enumerate() {
        let projected = match newest {
            0 => project_within(message, room),
            _ => project(message, MESSAGE_CONTENT_BYTES, MAX_TOOL_CALLS),
        };
        // Each element also costs a separating comma.
        let size = projected.to_string().len() + 1;
        match used.saturating_add(size) <= room {
            true => {
                used += size;
                kept.push((projected, message.ordinal));
            }
            false => break,
        }
    }
    let cut = page.messages.len() - kept.len();
    kept.reverse();
    let oldest_kept = kept.first().and_then(|(_, ordinal)| *ordinal);
    let (before, more) = match (cut > 0 || page.has_more_before, oldest_kept) {
        (true, Some(ordinal)) => (Some(ordinal.to_string()), true),
        // Nothing older, or nothing to name it by: nothing is offered.
        (true, None) | (false, _) => (None, false),
    };
    let messages: Vec<serde_json::Value> = kept.into_iter().map(|(value, _)| value).collect();
    envelope(&messages, before, more)
}

/// The newest message, cut down until its projection fits in `room`.
/// It starts whole (#2192 review: a one-message page reads a long final
/// report whole, up to the answer's budget) and is halved until it fits.
fn project_within(message: &Message, room: usize) -> serde_json::Value {
    let mut content = message.content.len().max(MESSAGE_CONTENT_BYTES);
    let mut calls = MAX_TOOL_CALLS;
    loop {
        let projected = project(message, content, calls);
        match (projected.to_string().len() < room, content, calls) {
            (true, _, _) | (false, 0, 0) => return projected,
            (false, 0, _) => calls = 0,
            (false, _, _) => content /= 2,
        }
    }
}

fn project(message: &Message, content_bytes: usize, tool_calls: usize) -> serde_json::Value {
    let (content, content_truncated) = bounded(&message.content, content_bytes);
    let calls: Vec<serde_json::Value> = message
        .tool_calls
        .iter()
        .take(tool_calls)
        .map(|call| {
            serde_json::json!({
                "id": bounded(&call.id, NAME_BYTES).0,
                "name": bounded(&call.name, NAME_BYTES).0,
                "arguments": bounded(&call.arguments, ARGUMENT_BYTES).0,
            })
        })
        .collect();
    let mut value = serde_json::json!({
        "ordinal": message.ordinal,
        "role": message.role.as_str(),
        "content": content,
    });
    let fields = value
        .as_object_mut()
        .expect("a message projects to an object");
    if content_truncated {
        fields.insert("contentLength".into(), message.content.len().into());
    }
    if let [_, ..] = calls.as_slice() {
        fields.insert("toolCalls".into(), calls.into());
    }
    if message.tool_calls.len() > tool_calls {
        fields.insert("toolCallCount".into(), message.tool_calls.len().into());
    }
    if let Some(id) = &message.tool_call_id {
        fields.insert("toolCallId".into(), bounded(id, NAME_BYTES).0.into());
    }
    if let Some(name) = &message.tool_name {
        fields.insert("toolName".into(), bounded(name, NAME_BYTES).0.into());
    }
    if message.is_error {
        fields.insert("isError".into(), true.into());
    }
    // #2404 review M2: a watermark cut's stub is told from a prompt.
    if let Some(kind) = crate::infrastructure::turn_origin_names::user_kind_name(message.user_kind)
    {
        fields.insert("userKind".into(), kind.into());
    }
    value
}

/// `text` cut on a character boundary to at most `max` bytes, and whether
/// it was cut.
fn bounded(text: &str, max: usize) -> (&str, bool) {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], end < text.len())
}

fn ok(command: &str, data: serde_json::Value) -> ToolResult {
    ToolResult {
        content: serde_json::json!({
            "type": "response",
            "command": command,
            "success": true,
            "data": data,
        })
        .to_string(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

fn error(message: String) -> ToolResult {
    ToolResult {
        content: format!("agent_cmd error: {message}"),
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

#[path = "agent_cmd_ended_report.rs"]
mod report;

#[cfg(test)]
#[path = "agent_cmd_ended_provenance_tests.rs"]
mod provenance_tests;
#[cfg(test)]
#[path = "agent_cmd_ended_tests.rs"]
mod tests;
