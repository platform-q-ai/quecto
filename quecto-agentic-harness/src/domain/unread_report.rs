//! Default unread-report policy of a supervised transcript (#1856, #1971):
//! which of a child's messages a plain `get_messages` (no explicit page)
//! reports to its supervisor, given the durable ordinal the supervisor last
//! acknowledged, and when a report advances that watermark.
//!
//! Pure rules over typed observations of the messages; the `agent_cmd`
//! adapter parses the child's wire response into them, applies these
//! rules, and keeps the transport budget and envelope shaping to itself.
use super::session::PendingMessageReport;
use super::turn_origin::{TurnOrigin, report_index};
use std::collections::VecDeque;

/// What the policy needs to know about one reported message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportedMessage {
    /// The durable ordinal persistence assigned, if any yet.
    pub ordinal: Option<u64>,
    /// An assistant message with non-blank content and no tool calls: a
    /// report the supervisor can act on.
    pub substantive_assistant: bool,
    /// What opened the message's turn: a nudge turn's reply never replaces
    /// the report (#2226).
    pub origin: TurnOrigin,
}

/// The messages a default report covers, by index into the observed list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnreadSelection {
    /// A live turn published messages persistence has not yet numbered:
    /// expose the unread report ([`report_among`]) by id without inventing a
    /// watermark (cursor-neutral, incomplete).
    PendingPersistence { report: Option<usize> },
    /// Nothing newer than the watermark.
    Unchanged,
    /// The child's transcript could not be read completely: the unread
    /// messages after the watermark (possibly none), reported incomplete
    /// and never acknowledged; `max_ordinal` is the newest observed.
    Incomplete {
        unread: Vec<usize>,
        max_ordinal: u64,
    },
    /// The unread messages to report and acknowledge: the report
    /// ([`report_among`]) alone on a first read (`delivered == 0`), every
    /// unread message afterwards. `max_ordinal` is the newest durable
    /// ordinal the child holds, at least the watermark.
    Unread {
        indices: Vec<usize>,
        max_ordinal: u64,
    },
}

/// Select what a default report covers, given the observed messages in
/// transcript order, the acknowledged watermark `delivered`, and whether
/// the observation itself was incomplete.
pub fn select_unread(
    messages: &[ReportedMessage],
    delivered: u64,
    report_incomplete: bool,
) -> UnreadSelection {
    select(messages, delivered, report_incomplete, FirstRead::Report)
}

/// What a first read (`delivered == 0`) delivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstRead {
    /// The report alone ([`report_among`]).
    Report,
    /// Every unread message: the reader found no report to deliver.
    Window,
}

/// [`select_unread`] for a first read that looked for a report as far back
/// as it may page and found none (#2246): every unread message it holds is
/// delivered and acknowledged like a later read's, so the next read moves on
/// instead of paging back for an answer that is not there. Any other read
/// selects as [`select_unread`] does.
pub fn select_unread_without_report(
    messages: &[ReportedMessage],
    delivered: u64,
    report_incomplete: bool,
) -> UnreadSelection {
    select(messages, delivered, report_incomplete, FirstRead::Window)
}

fn select(
    messages: &[ReportedMessage],
    delivered: u64,
    report_incomplete: bool,
    first_read: FirstRead,
) -> UnreadSelection {
    if messages.iter().any(|m| m.ordinal.is_none()) {
        // Only what the supervisor has not read yet: a live nudge turn must
        // not bring back an answer it already acknowledged (#2226).
        let unread: Vec<usize> = (0..messages.len())
            .filter(|&i| {
                messages[i]
                    .ordinal
                    .is_none_or(|ordinal| ordinal > delivered)
            })
            .collect();
        return UnreadSelection::PendingPersistence {
            report: report_among(messages, &unread),
        };
    }
    let ordinal = |index: usize| messages[index].ordinal.unwrap_or(0);
    let observed_max = (0..messages.len()).map(ordinal).max().unwrap_or(0);
    let unread: Vec<usize> = (0..messages.len())
        .filter(|&i| ordinal(i) > delivered)
        .collect();
    if report_incomplete {
        return UnreadSelection::Incomplete {
            unread,
            max_ordinal: observed_max,
        };
    }
    if observed_max < delivered || unread.is_empty() {
        return UnreadSelection::Unchanged;
    }
    let indices = match (delivered, first_read) {
        (0, FirstRead::Report) => report_among(messages, &unread).into_iter().collect(),
        (0, FirstRead::Window) | (1.., _) => unread,
    };
    if indices.is_empty() {
        return UnreadSelection::Unchanged;
    }
    UnreadSelection::Unread {
        indices,
        max_ordinal: delivered.max(observed_max),
    }
}

/// The report among the `candidates` (indices into `messages`): the latest
/// substantive answer to an instruction, never a nudge turn's reply while
/// one exists (#2226); its index into `messages`.
pub fn report_among(messages: &[ReportedMessage], candidates: &[usize]) -> Option<usize> {
    report_index(
        candidates.len(),
        |p| messages[candidates[p]].substantive_assistant,
        |p| messages[candidates[p]].origin,
    )
    .map(|position| candidates[position])
}

/// How many messages the child holds after the one at `report`, by durable
/// ordinal (the newest observed less the report's): the progress a first
/// read leaves unread (#2226), counted even where the page does not reach.
pub fn later_than(messages: &[ReportedMessage], report: usize) -> u64 {
    let after = messages[report].ordinal.unwrap_or(0);
    debug_assert!(
        messages[report].ordinal.is_some(),
        "a delivered report is numbered"
    );
    let newest = messages
        .iter()
        .filter_map(|m| m.ordinal)
        .max()
        .unwrap_or(after);
    newest.saturating_sub(after)
}

/// Whether a default report must page further back before it can be
/// shaped: older history exists (`has_older`) and the page's smallest
/// durable ordinal (`oldest_ordinal`) lies above the watermark by a gap —
/// unless this is a first read (`delivered == 0`) that already holds an
/// assistant message of an instruction turn (`holds_answer`, #2226), which
/// reports the latest answer without backfilling.
/// A page with nothing older is the whole transcript (#2218): its order
/// need not follow its ordinals (a recall notice inserted at the head after
/// the task was saved is numbered after it).
pub fn needs_backfill(
    oldest_ordinal: Option<u64>,
    holds_answer: bool,
    delivered: u64,
    has_older: bool,
) -> bool {
    let gap = oldest_ordinal.is_some_and(|ordinal| ordinal > delivered.saturating_add(1));
    let reports_latest_without_backfill = delivered == 0 && holds_answer;
    has_older && gap && !reports_latest_without_backfill
}

/// The pending report a delivered result acknowledges: by receipt when the
/// delivery carried one, else by exact content — and only when exactly one
/// receipt-less report matches, so an ambiguous match acknowledges nothing.
pub fn acknowledged_report_index(
    pending: &VecDeque<PendingMessageReport>,
    receipt: Option<&str>,
    delivered_content: &str,
) -> Option<usize> {
    if let Some(receipt) = receipt {
        return pending
            .iter()
            .position(|pending| !pending.receipt.is_empty() && pending.receipt == receipt);
    }
    let mut matches = pending
        .iter()
        .enumerate()
        .filter(|(_, pending)| pending.receipt.is_empty() && pending.response == delivered_content);
    let (index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(index)
}

#[cfg(test)]
#[path = "unread_report_tests.rs"]
mod tests;
