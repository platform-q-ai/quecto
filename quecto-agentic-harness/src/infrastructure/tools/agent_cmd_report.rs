//! The `agent_cmd` default unread report (#1856): the adapter side of the
//! supervisor's plain `get_messages` read. The child's wire response is
//! parsed here, the selection and acknowledgement rules are the domain's
//! (`domain::unread_report`), and the report budget, receipt and envelope
//! shaping stay with this tool.
use crate::domain::session::PendingMessageReport;
use crate::domain::turn_origin::{TurnOrigin, report_index};
use crate::domain::unread_report::{
    ReportedMessage, UnreadSelection, acknowledged_report_index, later_than, needs_backfill,
    select_unread, select_unread_without_report,
};

pub(crate) fn mint_default_report_receipt() -> String {
    format!("agent-cmd-report-{}", uuid::Uuid::new_v4())
}

pub(crate) fn delivery_receipt(response: &serde_json::Value) -> Option<&str> {
    response
        .pointer("/data/deliveryReceipt")
        .and_then(|v| v.as_str())
        .filter(|receipt| !receipt.is_empty())
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DefaultReportPlan {
    pub content: String,
    pub pending: Option<PendingMessageReport>,
}

fn ordinal_of(message: &serde_json::Value) -> Option<u64> {
    message.get("ordinal").and_then(|value| value.as_u64())
}

/// What opened the message's turn, as the child marked it (#2226).
pub(crate) fn turn_origin_of(message: &serde_json::Value) -> TurnOrigin {
    crate::infrastructure::turn_origin_names::origin_from_name(
        message.get("turnOrigin").and_then(|v| v.as_str()),
    )
}

/// The child's report among `messages`: the latest substantive answer to
/// an instruction, never a nudge turn's reply while one exists (#2226).
pub(crate) fn report_position(messages: &[serde_json::Value]) -> Option<usize> {
    report_index(
        messages.len(),
        |i| is_substantive_assistant(&messages[i]),
        |i| turn_origin_of(&messages[i]),
    )
}

fn observed(message: &serde_json::Value) -> ReportedMessage {
    ReportedMessage {
        ordinal: ordinal_of(message),
        substantive_assistant: is_substantive_assistant(message),
        origin: turn_origin_of(message),
    }
}

pub(crate) fn plan_default_report(response: &str, delivered: u64) -> DefaultReportPlan {
    let unchanged = |content: String| DefaultReportPlan {
        content,
        pending: None,
    };
    let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(response) else {
        return unchanged(response.to_string());
    };
    if envelope.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return unchanged(response.to_string());
    }
    let Some(data) = envelope.get_mut("data") else {
        return unchanged(response.to_string());
    };
    let report_incomplete = data.get("reportIncomplete").and_then(|v| v.as_bool()) == Some(true);
    // A first read that paged back as far as it may without finding a
    // report says so (#2246): it delivers the window it holds instead.
    let no_report_found = data.get("reportFound").and_then(|v| v.as_bool()) == Some(false);
    let older_unread_skipped = data.get("olderUnreadSkipped").cloned();
    let Some(messages) = data.get_mut("messages").and_then(|v| v.as_array_mut()) else {
        return unchanged(response.to_string());
    };
    let selected = |indices: &[usize]| -> Vec<serde_json::Value> {
        indices
            .iter()
            .map(|&index| messages[index].clone())
            .collect()
    };
    let reported: Vec<ReportedMessage> = messages.iter().map(observed).collect();
    let selection = match no_report_found {
        true => select_unread_without_report(&reported, delivered, report_incomplete),
        false => select_unread(&reported, delivered, report_incomplete),
    };
    match selection {
        UnreadSelection::PendingPersistence { report } => {
            let report =
                bounded_report_messages(selected(&report.into_iter().collect::<Vec<_>>()), 0);
            *data = serde_json::json!({"messages":report.messages, "cursorNeutral":true,
                "ordinalStatus":"pending_persistence", "reportIncomplete":true,
                "messageContentTruncated":report.message_content_truncated});
            unchanged(envelope.to_string())
        }
        UnreadSelection::Incomplete {
            unread,
            max_ordinal,
        } => {
            if !unread.is_empty() {
                let report = bounded_report_messages(selected(&unread), max_ordinal);
                if !report.messages.is_empty() {
                    *data = serde_json::json!({"messages": report.messages, "truncated": true, "hasMoreMessages": report.has_more_messages, "messageContentTruncated": report.message_content_truncated, "reportIncomplete": true});
                    return unchanged(envelope.to_string());
                }
            }
            *data = serde_json::json!({"unchanged": true, "reportIncomplete": true});
            unchanged(envelope.to_string())
        }
        UnreadSelection::Unchanged => {
            *data = serde_json::json!({"unchanged": true});
            unchanged(envelope.to_string())
        }
        UnreadSelection::Unread {
            indices,
            max_ordinal,
        } => {
            let report = bounded_report_messages(selected(&indices), max_ordinal);
            if report.messages.is_empty() {
                *data = if max_ordinal > delivered {
                    serde_json::json!({"messages": [], "truncated": true, "hasMoreMessages": true, "messageContentTruncated": false, "reportIncomplete": true})
                } else {
                    serde_json::json!({"unchanged": true})
                };
                return unchanged(envelope.to_string());
            }
            let ordinal = report
                .messages
                .iter()
                .filter_map(ordinal_of)
                .max()
                .unwrap_or(delivered);
            let truncated = report.has_more_messages || report.message_content_truncated;
            let receipt = mint_default_report_receipt();
            *data = serde_json::json!({"messages": report.messages, "truncated": truncated, "hasMoreMessages": report.has_more_messages, "messageContentTruncated": report.message_content_truncated});
            // A first read delivers the report alone: say how much newer
            // progress (a workflow's nudge turns) the next read brings (#2226).
            let later = match indices.as_slice() {
                [report] if delivered == 0 => later_than(&reported, *report),
                _ => 0,
            };
            if later > 0 {
                data["laterProgress"] = serde_json::json!(later);
            }
            // A later read past the backfill cap skipped older unread
            // history; it is still readable with explicit pages.
            if let Some(skipped) = older_unread_skipped {
                data["olderUnreadSkipped"] = skipped;
            }
            if no_report_found {
                data["reportFound"] = serde_json::json!(false);
            }
            let content = envelope.to_string();
            DefaultReportPlan {
                content: content.clone(),
                pending: Some(PendingMessageReport {
                    receipt,
                    response: content,
                    ordinal,
                }),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeliveryDecision {
    Ignore,
    Clear,
    Acknowledge(usize),
}

pub(crate) fn plan_delivery(
    command: Option<&str>,
    explicit_page: bool,
    result_is_error: bool,
    content: &str,
    metadata_receipt: Option<&str>,
    pending: &std::collections::VecDeque<PendingMessageReport>,
) -> DeliveryDecision {
    if result_is_error {
        return DeliveryDecision::Ignore;
    }
    if command == Some("clear_history") {
        return if serde_json::from_str::<serde_json::Value>(content)
            .ok()
            .and_then(|v| v.get("success").and_then(|v| v.as_bool()))
            == Some(true)
        {
            DeliveryDecision::Clear
        } else {
            DeliveryDecision::Ignore
        };
    }
    if command != Some("get_messages") || explicit_page {
        return DeliveryDecision::Ignore;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return DeliveryDecision::Ignore;
    };
    if value.get("success").and_then(|v| v.as_bool()) != Some(true)
        || value
            .pointer("/data/reportIncomplete")
            .and_then(|v| v.as_bool())
            == Some(true)
    {
        return DeliveryDecision::Ignore;
    }
    let receipt = metadata_receipt.or_else(|| delivery_receipt(&value));
    acknowledged_report_index(pending, receipt, content)
        .map(DeliveryDecision::Acknowledge)
        .unwrap_or(DeliveryDecision::Ignore)
}

/// What a child's page says of its report (#2226).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PageReport {
    /// The page carries no `report` field, or one this reader cannot use:
    /// the page alone must be read (paging back as before #2226).
    Unnamed,
    /// The child has no report (`report: null`): there is nothing to page
    /// back for.
    NamesNone,
    /// The page names its report and holds it.
    OnPage,
    /// The page names a report it does not hold: read it by id.
    OffPage(NamedReport),
}

/// A report named off the page: what the child said of it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NamedReport {
    pub id: String,
    ordinal: serde_json::Value,
    turn_origin: serde_json::Value,
}

impl NamedReport {
    /// The report as a delivered message, from its text read from the child
    /// (`text`, of the full `length`): never a stand-in (#2226 review 2). A
    /// text read only in part (past the final-report budget) is not marked
    /// for reading again: the report budget cuts it once, with its notice.
    pub(crate) fn with_text(&self, text: String, length: usize) -> serde_json::Value {
        debug_assert!(text.len() == length || text.len() > FINAL_REPORT_BUDGET_BYTES);
        serde_json::json!({"id": self.id, "role": "assistant", "content": text,
            "ordinal": self.ordinal, "turnOrigin": self.turn_origin, "contentLength": length})
    }

    /// Put the report, read whole, into `messages`: in place of its collapsed
    /// stub when the page holds one, else before the oldest message.
    pub(crate) fn place(&self, messages: &mut Vec<serde_json::Value>, report: serde_json::Value) {
        let stub = messages
            .iter()
            .position(|m| m.get("id").and_then(|v| v.as_str()) == Some(self.id.as_str()));
        match stub {
            Some(index) => messages[index] = report,
            None => messages.insert(0, report),
        }
    }

    /// Whether the supervisor has yet to read the report (above the
    /// `delivered` watermark, or not yet numbered).
    pub(crate) fn is_unread(&self, delivered: u64) -> bool {
        self.ordinal
            .as_u64()
            .is_none_or(|ordinal| ordinal > delivered)
    }
}

/// Whether `messages` hold message `id` whole: one the child collapsed to a
/// recall stub is read by id like one off the page (#2226 review 3).
pub(crate) fn holds_whole(messages: &[serde_json::Value], id: &str) -> bool {
    messages.iter().any(|m| {
        m.get("id").and_then(|v| v.as_str()) == Some(id)
            && matches!(
                m.get("collapsed"),
                None | Some(serde_json::Value::Bool(false))
            )
    })
}

/// What `data`'s page says of its report, given the `messages` it holds. A
/// named report must carry a string `id`; anything else is unnamed.
pub(crate) fn page_report(messages: &[serde_json::Value], data: &serde_json::Value) -> PageReport {
    let Some(named) = data.get("report") else {
        return PageReport::Unnamed;
    };
    if named.is_null() {
        return PageReport::NamesNone;
    }
    let Some(id) = named.get("id").and_then(|v| v.as_str()) else {
        return PageReport::Unnamed;
    };
    if holds_whole(messages, id) {
        return PageReport::OnPage;
    }
    PageReport::OffPage(NamedReport {
        id: id.to_string(),
        ordinal: named.get("ordinal").cloned().unwrap_or_default(),
        turn_origin: named.get("turnOrigin").cloned().unwrap_or_default(),
    })
}

/// Whether the default report must page older history in: `has_older` is
/// the oldest page's own `hasMoreBefore`.
pub(crate) fn needs_default_report_backfill(
    messages: &[serde_json::Value],
    delivered: u64,
    has_older: bool,
) -> bool {
    // A first read stops at a substantive answer: to an instruction, or
    // unmarked (#2226). A page from a child older than #2226 marks nothing:
    // any assistant message stops it, as before, so an old child is not
    // paged back to the cap.
    let marked = messages.iter().any(|m| m.get("turnOrigin").is_some());
    let holds_answer = messages.iter().any(|m| match marked {
        true => is_substantive_assistant(m) && turn_origin_of(m).answers(),
        false => m.get("role").and_then(|v| v.as_str()) == Some("assistant"),
    });
    needs_backfill(
        messages.iter().filter_map(ordinal_of).min(),
        holds_answer,
        delivered,
        has_older,
    )
}

pub(crate) const REPORT_BUDGET_BYTES: usize = 800 * 4;
/// A finished child's final report is delivered whole up to this size
/// (#2114); beyond it the start is kept with a plain notice.
pub(crate) const FINAL_REPORT_BUDGET_BYTES: usize = 64 * 1024;

pub(crate) struct BoundedReport {
    pub messages: Vec<serde_json::Value>,
    pub has_more_messages: bool,
    pub message_content_truncated: bool,
}

pub(crate) fn bounded_report_messages(
    mut candidates: Vec<serde_json::Value>,
    max_available_ordinal: u64,
) -> BoundedReport {
    for msg in &mut candidates {
        strip_unbounded_payloads(msg);
    }

    // The child's report (#2226: never a nudge turn's reply while an
    // answer exists) is its handoff: it gets its own budget (#2114) and the
    // rest of the report, context, the ordinary one on top of it.
    let final_idx = report_position(&candidates);
    if let Some(index) = final_idx {
        let final_message = &mut candidates[index];
        if report_envelope_size(&[], Some(final_message)) > FINAL_REPORT_BUDGET_BYTES {
            // The notice first, so the cut makes room for it too.
            final_message["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
            truncate_message_to_fit(final_message, &[], FINAL_REPORT_BUDGET_BYTES);
        }
    }
    // A message the child itself collapsed (too large for its history page)
    // cannot be paged whole either; say where its text is.
    for (index, msg) in candidates.iter_mut().enumerate() {
        let collapsed = msg.get("collapsed").and_then(|v| v.as_bool()) == Some(true)
            && msg.get("truncated").and_then(|v| v.as_bool()) == Some(true);
        if collapsed && Some(index) != final_idx && msg.get("contentNotice").is_none() {
            msg["contentNotice"] = serde_json::json!(COLLAPSED_PREVIEW_NOTICE);
        }
    }
    let context_cut_notice = if final_idx.is_some() {
        CONTEXT_CUT_NOTICE
    } else {
        PROGRESS_CUT_NOTICE
    };
    let budget = final_idx
        .map(|index| report_envelope_size(&[], Some(&candidates[index])))
        .unwrap_or(0)
        + REPORT_BUDGET_BYTES;

    // Preserve transcript order when it already fits. If it does not, reserve the
    // envelope for the latest substantive assistant handoff before optional context.
    let all_fit = report_envelope_size(&candidates, None) <= budget;
    let mut ordered = if all_fit {
        candidates
    } else if let Some(final_idx) = final_idx {
        let final_message = candidates.remove(final_idx);
        let mut prioritized = vec![final_message];
        prioritized.extend(candidates.into_iter().rev());
        prioritized
    } else {
        // During a long tool-only turn, report current progress rather than
        // forcing the supervisor to page through the oldest unread tools.
        candidates.into_iter().rev().collect()
    };

    let candidate_count = ordered.len();
    let mut selected = Vec::new();
    for mut msg in ordered.drain(..) {
        if report_envelope_size(&selected, Some(&msg)) > budget {
            truncate_message_to_fit(&mut msg, &selected, budget);
            if msg.get("contentNotice").is_none() {
                msg["contentNotice"] = serde_json::json!(context_cut_notice);
                if report_envelope_size(&selected, Some(&msg)) > budget {
                    truncate_message_to_fit(&mut msg, &selected, budget);
                }
            }
        }
        if is_emptied_by_truncation(&msg) {
            break;
        }
        if report_envelope_size(&selected, Some(&msg)) <= budget {
            selected.push(msg);
        } else {
            break;
        }
    }
    if !all_fit {
        selected.sort_by_key(|m| m.get("ordinal").and_then(|v| v.as_u64()).unwrap_or(0));
    }
    let committed = selected
        .iter()
        .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
        .max()
        .unwrap_or(0);
    let message_content_truncated = selected
        .iter()
        .any(|m| m.get("truncated").and_then(|v| v.as_bool()) == Some(true));
    BoundedReport {
        has_more_messages: selected.len() < candidate_count || committed < max_available_ordinal,
        message_content_truncated,
        messages: selected,
    }
}

/// Shown on a context message cut to fit beside the final report.
pub(crate) const CONTEXT_CUT_NOTICE: &str =
    "Cut to fit beside the final report; read it in full with get_messages count/before.";

/// Shown on a message cut to fit a progress report (no final report yet).
pub(crate) const PROGRESS_CUT_NOTICE: &str =
    "Cut to fit the report; read it in full with get_messages count/before.";

/// Shown on a message the child collapsed because it is too large for a
/// history page: paging returns the same preview.
pub(crate) const COLLAPSED_PREVIEW_NOTICE: &str = "Only a preview: this message is too large to read through get_messages. Call agent_cmd get_report with export_raw true to write the full history to an artifact.";

/// Shown on a final report cut at [`FINAL_REPORT_BUDGET_BYTES`].
pub(crate) const FINAL_REPORT_NOTICE: &str = "This report is longer than the report budget and was cut; its start is shown. Call agent_cmd get_report with export_raw true to write the child's full retained history to an artifact you can read.";

pub(crate) fn is_substantive_assistant(msg: &serde_json::Value) -> bool {
    msg.get("role").and_then(|v| v.as_str()) == Some("assistant")
        && msg
            .get("content")
            .and_then(|v| v.as_str())
            .is_some_and(|content| !content.trim().is_empty())
        && msg
            .get("toolCalls")
            .and_then(|v| v.as_array())
            .is_none_or(Vec::is_empty)
        && msg
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_none_or(Vec::is_empty)
}

/// A message truncation left with no content cannot be delivered.
fn is_emptied_by_truncation(msg: &serde_json::Value) -> bool {
    msg.get("truncated").and_then(|v| v.as_bool()) == Some(true)
        && msg
            .get("content")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
}

fn report_envelope_size(selected: &[serde_json::Value], next: Option<&serde_json::Value>) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "messages": selected.iter().chain(next).collect::<Vec<_>>(),
        "truncated": false,
        "hasMoreMessages": false,
        "messageContentTruncated": false,
    }))
    .map(|v| v.len())
    .unwrap_or(usize::MAX)
}

fn strip_unbounded_payloads(msg: &mut serde_json::Value) {
    if let Some(obj) = msg.as_object_mut() {
        obj.remove("toolCalls");
        obj.remove("tool_calls");
        obj.remove("imageBlocks");
        obj.remove("image_blocks");
    }
}

/// Cut `msg`'s content to the longest prefix (on a character boundary)
/// that keeps the report envelope within `budget`. The cut is marked
/// `truncated` with the original `contentLength`; no recovery command is
/// offered (#2114).
fn truncate_message_to_fit(
    msg: &mut serde_json::Value,
    selected: &[serde_json::Value],
    budget: usize,
) {
    let Some(original) = msg
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        msg["truncated"] = serde_json::json!(true);
        return;
    };
    // A length the child already reported (a preview's) is the true one.
    if msg.get("contentLength").is_none() {
        msg["contentLength"] = serde_json::json!(original.len());
    }
    msg["truncated"] = serde_json::json!(true);
    if let Some(obj) = msg.as_object_mut() {
        obj.remove("contentRecovery");
    }
    let mut low = 0usize;
    let mut high = original.len();
    let mut best = 0usize;
    while low <= high {
        let mid = (low + high) / 2;
        let mut end = mid.min(original.len());
        while !original.is_char_boundary(end) {
            end -= 1;
        }
        msg["content"] = serde_json::Value::String(original[..end].to_string());
        if report_envelope_size(selected, Some(msg)) <= budget {
            best = end;
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    msg["content"] = serde_json::Value::String(original[..best].to_string());
}
