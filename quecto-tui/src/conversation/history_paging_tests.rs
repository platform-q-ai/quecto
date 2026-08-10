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

/// Every field `reset` clears must be NON-DEFAULT before it runs, or the
/// assertions are vacuous: an earlier version seeded state whose `reconcile`
/// had already nulled `pending_page` and never latched `backfilled`, so
/// deleting either line from `reset` killed no test at all.
#[test]
fn reset_drops_every_cursor_and_the_in_flight_page() {
    let mut p = HistoryPaging::default();
    // Latch the backfill and record a prefix (a COMPLETE page), then take a
    // partial page so a cursor and an in-flight request also exist.
    p.reconcile(&facts(3, false, false, false));
    assert!(p.backfilled, "arrange: the guard must be latched");
    p.reconcile(&facts(2, true, false, true));
    assert_eq!(
        p.partial_prefix_len,
        Some(2),
        "arrange: a prefix is recorded"
    );
    let issued = request(&mut p, true, Instant::now()).expect("arrange: a page is in flight");
    assert!(p.is_pending_page(Some(&issued.request_id)));
    p.backfilled = true;

    p.reset();

    assert_eq!(p.pending_page, None, "an in-flight page must be forgotten");
    assert!(
        !p.is_pending_page(Some(&issued.request_id)),
        "its response must no longer correlate"
    );
    assert_eq!(p.before_cursor, None);
    assert!(!p.has_more_before);
    assert_eq!(p.partial_prefix_len, None);
    assert!(
        !p.backfilled,
        "a latched guard must be cleared, or the swapped conversation never \
         backfills again"
    );
    // The sequence deliberately survives: it keeps correlation ids unique
    // across a lifecycle reset, so a page still in flight from BEFORE the reset
    // can never correlate with one issued after it.
    assert!(
        p.page_seq > 0,
        "reset must not rewind the correlation sequence"
    );
}

/// `reset` must clear the latch even when no partial prefix is present — the
/// two fields are independent and were previously only ever cleared together.
#[test]
fn reset_clears_a_latched_guard_that_has_no_partial_prefix() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, false, false, false));
    assert!(p.backfilled && p.partial_prefix_len.is_none(), "arrange");

    p.reset();

    assert!(!p.backfilled, "the latch must be cleared on its own");
}

/// `reopen_backfill` must clear a NON-EMPTY partial prefix. Latching via a
/// complete page leaves `partial_prefix_len` already `None`, which made the
/// old assertion vacuous — deleting the line killed no test.
#[test]
fn reopen_backfill_keeps_cursors_but_unlatches_the_guard() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, true, false, false));
    assert_eq!(
        p.partial_prefix_len,
        Some(3),
        "arrange: a prefix is recorded"
    );
    p.backfilled = true;

    p.reopen_backfill();

    assert!(!p.backfilled);
    assert_eq!(
        p.partial_prefix_len, None,
        "a stale prefix length would feed the next ReplacePrefix and delete \
         live transcript entries below the backfill"
    );
    assert_eq!(
        p.before_cursor.as_deref(),
        Some("cursor"),
        "a wholesale replacement keeps the cursors its payload published"
    );
    assert!(
        p.has_more_before,
        "and keeps the advertised-more flag, so paging stays reachable"
    );
}

#[test]
fn wholesale_replacement_clears_stale_prefix_before_later_snapshot() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, true, false, false));
    p.reconcile(&facts(2, true, false, true));
    assert_eq!(
        p.partial_prefix_len,
        Some(5),
        "arrange: old chat has retained prefix"
    );

    p.reopen_backfill();
    assert_eq!(
        p.reconcile(&facts(2, true, false, false)),
        Some(PrefixPlan::Prepend),
        "after a wholesale replacement, a fresh partial snapshot must not use \
         the OLD transcript's prefix length as ReplacePrefix against the new chat"
    );
    assert_eq!(
        p.partial_prefix_len,
        Some(2),
        "the retained prefix now belongs to the replacement transcript only"
    );
}

#[test]
fn short_wholesale_replacement_is_not_wiped_by_longer_stale_prefix() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(6, true, false, false));
    assert_eq!(
        p.partial_prefix_len,
        Some(6),
        "arrange: stale prefix exceeds replacement"
    );

    p.reopen_backfill();
    assert_eq!(
        p.reconcile(&facts(1, true, false, false)),
        Some(PrefixPlan::Prepend),
        "a one-entry replacement must not produce ReplacePrefix(6), which \
         would remove the whole new transcript in the caller"
    );
    assert_eq!(p.partial_prefix_len, Some(1));
}

#[test]
fn wholesale_replacement_keeps_its_own_cursor_reachable_for_paging() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(4, true, false, false));
    assert_eq!(
        p.partial_prefix_len,
        Some(4),
        "arrange: old partial state exists"
    );

    p.reopen_backfill();
    assert_eq!(
        p.reconcile(&PageFacts {
            before: Some("replacement-cursor".into()),
            has_more_before: true,
            trimmed: false,
            page_len: 2,
            extend_prefix: false,
        }),
        Some(PrefixPlan::Prepend)
    );

    let issued = request(&mut p, true, Instant::now()).expect("replacement cursor pages");
    assert_eq!(issued.before, "replacement-cursor");
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

/// The `extend_prefix` × latch matrix had two unreachable-by-test corners
/// (#1236 review). Both are reachable from `app_subagent_stream`, and a
/// plausible refactoring mutation in either was invisible to the whole suite.
#[test]
fn an_own_older_page_that_closes_the_backfill_clears_the_partial_prefix() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, true, false, false));
    assert_eq!(p.partial_prefix_len, Some(3));

    // The final older page: no more history, not trimmed.
    assert_eq!(
        p.reconcile(&facts(2, false, false, true)),
        Some(PrefixPlan::Prepend),
        "an own older page still prepends"
    );
    assert!(p.backfilled, "closing the backfill must latch the guard");
    assert_eq!(
        p.partial_prefix_len, None,
        "a closed backfill must not leave a partial prefix behind"
    );
}

#[test]
fn a_partial_snapshot_over_a_partial_prefix_replaces_it_without_double_counting() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, true, false, false));
    p.reconcile(&facts(2, true, false, true));
    assert_eq!(p.partial_prefix_len, Some(5));

    // A fresh but still-incomplete snapshot replaces the whole loaded prefix.
    assert_eq!(
        p.reconcile(&facts(4, true, false, false)),
        Some(PrefixPlan::ReplacePrefix(5)),
        "a snapshot must replace every previously loaded page"
    );
    assert_eq!(
        p.partial_prefix_len,
        Some(4),
        "the new prefix is the SNAPSHOT's length; adding it to the replaced \
         length would double-count and make the next replace eat live entries"
    );
    assert!(!p.backfilled);
}

/// An empty page must not disturb a latch that a previous page established.
/// Previously this was only ever proven against a virgin struct.
#[test]
fn an_empty_page_does_not_disturb_an_already_latched_backfill() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, false, false, false));
    assert!(p.backfilled);

    assert_eq!(p.reconcile(&facts(0, false, false, false)), None);

    assert!(
        p.backfilled,
        "an empty broadcast snapshot must not un-latch a completed backfill \
         and re-trigger a full re-page"
    );
    assert_eq!(p.partial_prefix_len, None);
}

#[test]
fn an_empty_page_does_not_disturb_an_in_progress_partial_prefix() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, true, false, false));
    assert_eq!(p.partial_prefix_len, Some(3));

    assert_eq!(p.reconcile(&facts(0, true, false, false)), None);

    assert_eq!(
        p.partial_prefix_len,
        Some(3),
        "an empty page must not discard the loaded prefix count"
    );
    assert!(!p.backfilled);
}

/// The dedupe is per-CURSOR, not merely "something is in flight".
#[test]
fn a_pending_request_for_a_different_cursor_does_not_suppress_a_new_one() {
    let mut p = paging(Some("m9"), true);
    let start = Instant::now();
    let first = request(&mut p, true, start).expect("first request");
    assert_eq!(first.before, "m9");

    // The cursor advances after the page lands; a new page must issue at once.
    p.before_cursor = Some("m4".into());
    let second = request(&mut p, true, start)
        .expect("a different cursor must not be suppressed by the in-flight page");
    assert_eq!(second.before, "m4");
    assert!(
        !p.is_pending_page(Some(&first.request_id)),
        "the superseded request must no longer correlate"
    );
}

#[test]
fn no_id_correlates_when_nothing_is_in_flight() {
    let p = paging(Some("m5"), true);
    assert!(!p.is_pending_page(Some("history-page-anything-1")));
    assert!(!p.is_pending_page(None));
}

/// The final two corners of the `extend_prefix` × `partial_prefix_len` ×
/// latch matrix (#1236 round 2): an own older page arriving when NO partial
/// prefix is recorded. Reachable in production — an empty or fully filtered
/// snapshot leaves `partial_prefix_len = None` with `has_more_before = true`,
/// and the next own older page lands here.
#[test]
fn an_own_older_page_with_no_recorded_prefix_counts_only_its_own_length() {
    let mut p = HistoryPaging::default();
    // An empty snapshot publishes cursors but records no prefix.
    assert_eq!(p.reconcile(&facts(0, true, false, false)), None);
    assert_eq!(p.partial_prefix_len, None);

    assert_eq!(
        p.reconcile(&facts(4, true, false, true)),
        Some(PrefixPlan::Prepend),
        "an own older page prepends"
    );
    assert_eq!(
        p.partial_prefix_len,
        Some(4),
        "with no prefix recorded the count is the page length alone; any \
         non-zero base would over-count and make the next snapshot's \
         ReplacePrefix eat live transcript entries below the backfill"
    );
    assert!(!p.backfilled);
}

#[test]
fn an_own_older_page_with_no_recorded_prefix_that_closes_the_backfill() {
    let mut p = HistoryPaging::default();
    assert_eq!(p.reconcile(&facts(0, true, false, false)), None);
    assert_eq!(p.partial_prefix_len, None);

    assert_eq!(
        p.reconcile(&facts(4, false, false, true)),
        Some(PrefixPlan::Prepend)
    );
    assert!(p.backfilled, "closing the backfill must latch the guard");
    assert_eq!(
        p.partial_prefix_len, None,
        "a closed backfill leaves no partial prefix"
    );
}

/// A latched backfill receiving a partial page must be UN-latched. This is the
/// only path on which `reconcile`'s keep-open arm can observably change
/// `backfilled`, so without it that assignment is unpinned: deleting it kept
/// the whole suite green.
///
/// Reachable in production: an attach completes the backfill, the session is
/// re-attached, and the fresh snapshot is trimmed or advertises more history.
#[test]
fn a_partial_page_unlatches_a_previously_completed_backfill() {
    let mut p = HistoryPaging::default();
    p.reconcile(&facts(3, false, false, false));
    assert!(p.backfilled, "arrange: a complete page latched the guard");

    assert_eq!(
        p.reconcile(&facts(2, true, false, false)),
        Some(PrefixPlan::Prepend)
    );

    assert!(
        !p.backfilled,
        "a page advertising more history must reopen the backfill, or every \
         later attach snapshot is suppressed and the older history is stranded"
    );
    assert_eq!(p.partial_prefix_len, Some(2));
}
