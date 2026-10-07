use super::{
    ReportedMessage, UnreadSelection, acknowledged_report_index, needs_backfill, select_unread,
};
use crate::domain::sessions::entities::session::PendingMessageReport;
use crate::domain::turn_origin::TurnOrigin;
use std::collections::VecDeque;

fn numbered(ordinal: u64, substantive: bool) -> ReportedMessage {
    ReportedMessage {
        ordinal: Some(ordinal),
        substantive_assistant: substantive,
        origin: TurnOrigin::Instruction,
    }
}

/// A message of a workflow progress-nudge turn (#2226).
fn nudged(ordinal: u64, substantive: bool) -> ReportedMessage {
    ReportedMessage {
        origin: TurnOrigin::ProgressNudge,
        ..numbered(ordinal, substantive)
    }
}

fn unnumbered(substantive: bool, origin: TurnOrigin) -> ReportedMessage {
    ReportedMessage {
        ordinal: None,
        substantive_assistant: substantive,
        origin,
    }
}

#[test]
fn a_first_read_reports_only_the_latest_substantive_assistant() {
    let messages = [
        numbered(1, false),
        numbered(2, true),
        numbered(3, false),
        numbered(4, true),
    ];
    assert_eq!(
        select_unread(&messages, 0, false),
        UnreadSelection::Unread {
            indices: vec![3],
            max_ordinal: 4
        }
    );
    // Tool-only progress on a first read is not a report.
    assert_eq!(
        select_unread(&[numbered(1, false), numbered(2, false)], 0, false),
        UnreadSelection::Unchanged
    );
}

#[test]
fn later_reads_cover_every_unread_message_and_stay_unchanged_below_the_watermark() {
    let messages = [
        numbered(1, false),
        numbered(2, true),
        numbered(3, false),
        numbered(4, true),
    ];
    assert_eq!(
        select_unread(&messages, 2, false),
        UnreadSelection::Unread {
            indices: vec![2, 3],
            max_ordinal: 4
        }
    );
    assert_eq!(
        select_unread(&messages, 4, false),
        UnreadSelection::Unchanged
    );
    assert_eq!(
        select_unread(&messages, 9, false),
        UnreadSelection::Unchanged
    );
    assert_eq!(select_unread(&[], 0, false), UnreadSelection::Unchanged);
}

#[test]
fn unnumbered_messages_defer_to_pending_persistence() {
    let messages = [
        numbered(1, true),
        unnumbered(true, TurnOrigin::Instruction),
        unnumbered(false, TurnOrigin::Instruction),
        unnumbered(true, TurnOrigin::ProgressNudge),
    ];
    assert_eq!(
        select_unread(&messages, 0, false),
        UnreadSelection::PendingPersistence { report: Some(1) },
        "a live nudge turn's reply does not replace the report either"
    );
    assert_eq!(
        select_unread(&[unnumbered(false, TurnOrigin::Instruction)], 5, true),
        UnreadSelection::PendingPersistence { report: None }
    );
}

#[test]
fn an_incomplete_observation_reports_the_unread_tail_without_a_watermark() {
    let messages = [numbered(5, false), numbered(6, true)];
    assert_eq!(
        select_unread(&messages, 4, true),
        UnreadSelection::Incomplete {
            unread: vec![0, 1],
            max_ordinal: 6
        }
    );
    assert_eq!(
        select_unread(&messages, 6, true),
        UnreadSelection::Incomplete {
            unread: vec![],
            max_ordinal: 6
        }
    );
}

#[test]
fn backfill_is_needed_only_across_a_gap_above_the_watermark() {
    assert!(!needs_backfill(Some(2), true, 0, true));
    assert!(needs_backfill(Some(2), false, 0, true));
    assert!(needs_backfill(Some(12), true, 10, true));
    assert!(!needs_backfill(Some(11), true, 10, true));
    assert!(!needs_backfill(None, false, 10, true));
}

/// #2218: a page with nothing older holds the whole transcript, whatever
/// its oldest ordinal: a failed first turn (`[system 2, user 1]`) or a
/// re-inserted recall notice numbered after the task is complete as is.
#[test]
fn a_page_with_no_older_history_never_needs_backfill() {
    assert!(!needs_backfill(Some(2), false, 0, false));
    assert!(!needs_backfill(Some(12), true, 10, false));
}

#[test]
fn acknowledgement_matches_by_receipt_then_by_unique_content() {
    let pending: VecDeque<PendingMessageReport> = VecDeque::from(vec![
        PendingMessageReport {
            receipt: "r-1".into(),
            response: "body".into(),
            ordinal: 3,
        },
        PendingMessageReport {
            receipt: String::new(),
            response: "legacy".into(),
            ordinal: 4,
        },
        PendingMessageReport {
            receipt: String::new(),
            response: "dup".into(),
            ordinal: 5,
        },
        PendingMessageReport {
            receipt: String::new(),
            response: "dup".into(),
            ordinal: 6,
        },
    ]);
    assert_eq!(
        acknowledged_report_index(&pending, Some("r-1"), "x"),
        Some(0)
    );
    assert_eq!(
        acknowledged_report_index(&pending, Some("r-9"), "body"),
        None
    );
    assert_eq!(acknowledged_report_index(&pending, None, "legacy"), Some(1));
    assert_eq!(
        acknowledged_report_index(&pending, None, "dup"),
        None,
        "an ambiguous content match acknowledges nothing"
    );
    assert_eq!(acknowledged_report_index(&pending, None, "body"), None);
}

/// #2226: a first read reports the child's answer to its instruction, not
/// the replies to the workflow nudges that followed it.
#[test]
fn a_first_read_reports_the_answer_not_a_later_nudge_reply() {
    let messages = [
        numbered(1, false),
        numbered(2, true),
        nudged(3, false),
        nudged(4, true),
        nudged(5, false),
        nudged(6, true),
    ];
    assert_eq!(
        select_unread(&messages, 0, false),
        UnreadSelection::Unread {
            indices: vec![1],
            max_ordinal: 6
        }
    );
    // A later instruction's answer is the report again.
    let answered_again = [
        numbered(1, true),
        nudged(2, true),
        numbered(3, false),
        numbered(4, true),
    ];
    assert_eq!(
        select_unread(&answered_again, 0, false),
        UnreadSelection::Unread {
            indices: vec![3],
            max_ordinal: 4
        }
    );
}

/// #2226: a child that never answered an instruction still reports: its
/// latest nudge reply.
#[test]
fn a_first_read_with_only_nudge_replies_reports_the_latest_of_them() {
    let messages = [numbered(1, false), nudged(2, true), nudged(3, true)];
    assert_eq!(
        select_unread(&messages, 0, false),
        UnreadSelection::Unread {
            indices: vec![2],
            max_ordinal: 3
        }
    );
    // Later reads still cover every unread message, nudge replies included.
    assert_eq!(
        select_unread(&messages, 1, false),
        UnreadSelection::Unread {
            indices: vec![1, 2],
            max_ordinal: 3
        }
    );
}

/// #2226: the first read maps the report's position among the unread
/// messages back to its index in the page: a message at the watermark
/// (ordinal 0 here) is not unread, so the two differ.
#[test]
fn a_first_read_reports_the_answer_by_its_index_in_the_page() {
    let messages = [
        numbered(0, true),
        numbered(1, false),
        numbered(2, true),
        nudged(3, true),
    ];
    assert_eq!(
        select_unread(&messages, 0, false),
        UnreadSelection::Unread {
            indices: vec![2],
            max_ordinal: 3
        }
    );
}

/// Review probe P2 (#2226): while a nudge turn is still unsaved, a read
/// after the answer was acknowledged reports the new progress, never the
/// acknowledged answer again.
#[test]
fn a_pending_read_never_brings_back_an_acknowledged_answer() {
    let messages = [
        numbered(1, false),
        numbered(2, true),
        nudged(3, false),
        nudged(4, true),
        unnumbered(false, TurnOrigin::ProgressNudge),
    ];
    assert_eq!(
        select_unread(&messages, 2, false),
        UnreadSelection::PendingPersistence { report: Some(3) }
    );
    // Nothing unread but the live nudge: no report.
    assert_eq!(
        select_unread(&messages, 4, false),
        UnreadSelection::PendingPersistence { report: None }
    );
    // Before any acknowledgement the answer is the report.
    assert_eq!(
        select_unread(&messages, 0, false),
        UnreadSelection::PendingPersistence { report: Some(1) }
    );
}

#[test]
fn later_than_counts_the_messages_after_the_report() {
    let messages = [
        numbered(1, false),
        numbered(2, true),
        nudged(3, false),
        nudged(4, true),
    ];
    assert_eq!(super::later_than(&messages, 1), 2);
    assert_eq!(super::later_than(&messages, 3), 0);
}

/// #2246 review finding 1: a first read that found no report delivers every
/// unread message it holds, acknowledgeable, instead of nothing; any other
/// read selects as a report read does.
#[test]
fn a_first_read_without_a_report_delivers_its_window() {
    use super::select_unread_without_report;
    let window = [
        numbered(500, false),
        numbered(501, false),
        numbered(1002, false),
    ];
    assert_eq!(select_unread(&window, 0, false), UnreadSelection::Unchanged);
    assert_eq!(
        select_unread_without_report(&window, 0, false),
        UnreadSelection::Unread {
            indices: vec![0, 1, 2],
            max_ordinal: 1002
        }
    );
    for (delivered, incomplete) in [(501, false), (0, true), (2000, false)] {
        assert_eq!(
            select_unread_without_report(&window, delivered, incomplete),
            select_unread(&window, delivered, incomplete),
            "delivered {delivered}, incomplete {incomplete}"
        );
    }
}

/// #2246 cold review L3: the unread ordinals a delivery acknowledges without
/// delivering them, as inclusive ranges: every ordinal above the watermark
/// up to the newest delivered that is not itself delivered.
#[test]
fn skipped_ranges_cover_every_unread_ordinal_not_delivered() {
    use super::skipped_unread;
    assert_eq!(skipped_unread(0, &[8, 9, 10]), [(1, 7)]);
    assert_eq!(
        skipped_unread(0, &[500, 501, 502, 1000, 1001, 1002, 501]),
        [(1, 499), (503, 999)]
    );
    assert_eq!(skipped_unread(4, &[5, 6]), []);
    assert_eq!(skipped_unread(4, &[7, 5]), [(6, 6)]);
    assert_eq!(skipped_unread(0, &[]), []);
    assert_eq!(
        skipped_unread(9, &[3, 12]),
        [(10, 11)],
        "read ordinals are never skipped"
    );
}
