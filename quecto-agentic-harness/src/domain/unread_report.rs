//! Default unread-report policy of a supervised transcript (#1856, #1971):
//! which of a child's messages a plain `get_messages` (no explicit page)
//! reports to its supervisor, given the durable ordinal the supervisor last
//! acknowledged, and when a report advances that watermark.
//!
//! Pure rules over typed observations of the messages; the `agent_cmd`
//! adapter parses the child's wire response into them, applies these
//! rules, and keeps the transport budget and envelope shaping to itself.
use super::session::PendingMessageReport;
use std::collections::VecDeque;

/// What the policy needs to know about one reported message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportedMessage {
    /// The durable ordinal persistence assigned, if any yet.
    pub ordinal: Option<u64>,
    /// An assistant message with non-blank content and no tool calls: a
    /// report the supervisor can act on.
    pub substantive_assistant: bool,
}

/// The messages a default report covers, by index into the observed list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnreadSelection {
    /// A live turn published messages persistence has not yet numbered:
    /// expose the latest substantive report by id without inventing a
    /// watermark (cursor-neutral, incomplete).
    PendingPersistence { latest_substantive: Option<usize> },
    /// Nothing newer than the watermark.
    Unchanged,
    /// The child's transcript could not be read completely: the unread
    /// messages after the watermark (possibly none), reported incomplete
    /// and never acknowledged; `max_ordinal` is the newest observed.
    Incomplete {
        unread: Vec<usize>,
        max_ordinal: u64,
    },
    /// The unread messages to report and acknowledge: the latest
    /// substantive report alone on a first read (`delivered == 0`), every
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
    if messages.iter().any(|m| m.ordinal.is_none()) {
        return UnreadSelection::PendingPersistence {
            latest_substantive: messages.iter().rposition(|m| m.substantive_assistant),
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
    let indices = if delivered == 0 {
        unread
            .iter()
            .rev()
            .find(|&&i| messages[i].substantive_assistant)
            .copied()
            .into_iter()
            .collect()
    } else {
        unread
    };
    if indices.is_empty() {
        return UnreadSelection::Unchanged;
    }
    UnreadSelection::Unread {
        indices,
        max_ordinal: delivered.max(observed_max),
    }
}

/// Whether a default report must page further back before it can be
/// shaped: the newest page starts after a gap above the watermark — unless
/// this is a first read (`delivered == 0`) that already holds an assistant
/// message, which reports the latest one without backfilling.
pub fn needs_backfill(first_ordinal: Option<u64>, holds_assistant: bool, delivered: u64) -> bool {
    if delivered == 0 && holds_assistant {
        return false;
    }
    first_ordinal.is_some_and(|ordinal| ordinal > delivered.saturating_add(1))
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
