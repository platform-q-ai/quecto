//! Pure policy tests (#1221 acceptance criterion 1).
//!
//! These construct the history-paging policy directly — no terminal, no
//! concrete client, no raw JSON, no Tokio runtime — which is what the
//! extraction was for. They pin the correlation and backfill-latch invariants
//! at the level they actually live at.

use super::*;
use std::time::{Duration, Instant};

fn paging(cursor: Option<&str>, has_more: bool) -> HistoryPaging {
    HistoryPaging {
        before_cursor: cursor.map(str::to_owned),
        has_more_before: has_more,
        ..HistoryPaging::default()
    }
}

fn request(p: &mut HistoryPaging, at_oldest: bool, now: Instant) -> Option<HistoryPageRequest> {
    p.next_page_request(at_oldest, now, |seq| format!("page-{seq}"))
}

#[test]
fn no_page_is_requested_without_advertised_older_history() {
    let mut p = paging(Some("m5"), false);
    assert_eq!(request(&mut p, true, Instant::now()), None);
}

#[test]
fn no_page_is_requested_without_a_cursor() {
    let mut p = paging(None, true);
    assert_eq!(request(&mut p, true, Instant::now()), None);
}

#[test]
fn no_page_is_requested_until_the_oldest_loaded_entry_is_reached() {
    let mut p = paging(Some("m5"), true);
    assert_eq!(request(&mut p, false, Instant::now()), None);
}

#[test]
fn a_same_cursor_request_in_flight_is_deduped_until_it_goes_stale() {
    let mut p = paging(Some("m5"), true);
    let start = Instant::now();
    let first = request(&mut p, true, start).expect("first request");

    assert_eq!(
        request(
            &mut p,
            true,
            start + PENDING_HISTORY_PAGE_RETRY - Duration::from_millis(1)
        ),
        None,
        "a fresh in-flight request must suppress a duplicate"
    );

    let retry = request(&mut p, true, start + PENDING_HISTORY_PAGE_RETRY)
        .expect("a stale request must be retryable");
    assert_ne!(
        retry.request_id, first.request_id,
        "the retry must mint a fresh correlation id"
    );
    assert_eq!(
        retry.before, first.before,
        "the retry targets the same cursor"
    );
    assert!(
        !p.is_pending_page(Some(&first.request_id)),
        "the presumed-lost twin must no longer correlate"
    );
}

#[test]
fn only_an_exact_request_id_correlates() {
    let mut p = paging(Some("m5"), true);
    let issued = request(&mut p, true, Instant::now()).expect("request");

    assert!(p.is_pending_page(Some(&issued.request_id)));
    assert!(
        !p.is_pending_page(None),
        "an id-less broadcast must not correlate"
    );
    assert!(
        !p.is_pending_page(Some(&format!("{}-extra", issued.request_id))),
        "a prefix-only match must not correlate"
    );
    assert!(
        !p.is_pending_page(Some("history-page-someone-else-1")),
        "a foreign client's page must not correlate"
    );
}

#[test]
fn rollback_clears_only_the_matching_request() {
    let mut p = paging(Some("m5"), true);
    let issued = request(&mut p, true, Instant::now()).expect("request");

    p.rollback_pending_page("history-page-someone-else-1");
    assert!(
        p.is_pending_page(Some(&issued.request_id)),
        "an unrelated rollback must not clear our in-flight page"
    );

    p.rollback_pending_page(&issued.request_id);
    assert!(!p.is_pending_page(Some(&issued.request_id)));
    assert!(
        request(&mut p, true, Instant::now()).is_some(),
        "a rolled-back cursor must be immediately retryable"
    );
}

fn facts(page_len: usize, has_more: bool, trimmed: bool, extend: bool) -> PageFacts {
    PageFacts {
        before: Some("cursor".into()),
        has_more_before: has_more,
        trimmed,
        page_len,
        extend_prefix: extend,
    }
}

#[test]
fn an_empty_page_publishes_cursors_without_latching_the_backfill() {
    let mut p = HistoryPaging::default();
    assert_eq!(p.reconcile(&facts(0, true, false, false)), None);
    assert_eq!(
        p.before_cursor.as_deref(),
        Some("cursor"),
        "an empty page must still publish its cursor so paging stays reachable"
    );
    assert!(p.has_more_before);
    assert!(
        !p.backfilled,
        "an empty/filtered page must never latch the backfill guard"
    );
}

#[test]
fn a_complete_page_latches_the_backfill_and_clears_the_partial_prefix() {
    let mut p = HistoryPaging::default();
    assert_eq!(
        p.reconcile(&facts(3, false, false, false)),
        Some(PrefixPlan::Prepend)
    );
    assert!(p.backfilled);
    assert_eq!(p.partial_prefix_len, None);
}

#[test]
fn a_trimmed_or_incomplete_page_leaves_the_backfill_open() {
    for (has_more, trimmed) in [(true, false), (false, true), (true, true)] {
        let mut p = HistoryPaging::default();
        p.reconcile(&facts(3, has_more, trimmed, false));
        assert!(
            !p.backfilled,
            "has_more={has_more} trimmed={trimmed} must leave the backfill open"
        );
        assert_eq!(p.partial_prefix_len, Some(3));
    }
}

#[test]
fn an_own_older_page_grows_the_loaded_prefix() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, true, false, false));
    assert_eq!(
        p.reconcile(&facts(2, true, false, true)),
        Some(PrefixPlan::Prepend),
        "this session's own older page prepends"
    );
    assert_eq!(
        p.partial_prefix_len,
        Some(5),
        "the loaded prefix must accumulate across pages"
    );
}

#[test]
fn a_later_snapshot_replaces_the_whole_loaded_prefix() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, true, false, false));
    p.reconcile(&facts(2, true, false, true));
    assert_eq!(
        p.reconcile(&facts(9, false, false, false)),
        Some(PrefixPlan::ReplacePrefix(5)),
        "a fresh snapshot must replace EVERY previously loaded page, not just \
         the most recent one, which would duplicate the newest slice and leave \
         an interior gap"
    );
    assert_eq!(p.partial_prefix_len, None);
    assert!(p.backfilled);
}

#[test]
fn reset_drops_every_cursor_and_the_in_flight_page() {
    let mut p = paging(Some("m5"), true);
    request(&mut p, true, Instant::now());
    p.reconcile(&facts(3, true, false, false));

    p.reset();

    assert_eq!(p.pending_page, None);
    assert_eq!(p.before_cursor, None);
    assert!(!p.has_more_before);
    assert_eq!(p.partial_prefix_len, None);
    assert!(!p.backfilled);
    // The sequence deliberately survives: it keeps correlation ids unique
    // across a lifecycle reset, so a page still in flight from BEFORE the reset
    // can never correlate with one issued after it.
    assert!(
        p.page_seq > 0,
        "reset must not rewind the correlation sequence"
    );
}

#[test]
fn reopen_backfill_keeps_cursors_but_unlatches_the_guard() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, false, false, false));
    assert!(p.backfilled);

    p.reopen_backfill();

    assert!(!p.backfilled);
    assert_eq!(p.partial_prefix_len, None);
    assert_eq!(
        p.before_cursor.as_deref(),
        Some("cursor"),
        "a wholesale replacement keeps the cursors its payload published"
    );
}

#[test]
fn the_page_sequence_wraps_instead_of_overflowing() {
    let mut p = HistoryPaging {
        page_seq: u64::MAX,
        ..paging(Some("m5"), true)
    };
    let issued = request(&mut p, true, Instant::now()).expect("request");
    assert_eq!(
        issued.request_id, "page-0",
        "the sequence must wrap, not panic"
    );
}
