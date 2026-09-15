use super::{
    ReportedMessage, UnreadSelection, acknowledged_report_index, needs_backfill, select_unread,
};
use crate::domain::session::PendingMessageReport;
use std::collections::VecDeque;

fn numbered(ordinal: u64, substantive: bool) -> ReportedMessage {
    ReportedMessage {
        ordinal: Some(ordinal),
        substantive_assistant: substantive,
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
        ReportedMessage {
            ordinal: None,
            substantive_assistant: true,
        },
        ReportedMessage {
            ordinal: None,
            substantive_assistant: false,
        },
    ];
    assert_eq!(
        select_unread(&messages, 0, false),
        UnreadSelection::PendingPersistence {
            latest_substantive: Some(1)
        }
    );
    assert_eq!(
        select_unread(
            &[ReportedMessage {
                ordinal: None,
                substantive_assistant: false
            }],
            5,
            true
        ),
        UnreadSelection::PendingPersistence {
            latest_substantive: None
        }
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
    assert!(!needs_backfill(Some(2), true, 0));
    assert!(needs_backfill(Some(2), false, 0));
    assert!(needs_backfill(Some(12), true, 10));
    assert!(!needs_backfill(Some(11), true, 10));
    assert!(!needs_backfill(None, false, 10));
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
