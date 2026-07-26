//! Conversation history paging policy (#1221).
//!
//! A pure state machine owning the master/child history cursors, older-page
//! correlation, retry, and the partial-vs-complete backfill latch. It holds no
//! terminal, client, JSON, or runtime: callers pass in the few facts it cannot
//! know (whether the view is scrolled to the oldest loaded entry, whether a
//! sub-agent is focused, the current instant) and receive a decision back.
//!
//! Keeping this policy pure is what lets the correlation and backfill
//! invariants be exercised directly, instead of only through a rendered TUI.

use std::time::{Duration, Instant};

/// Re-request a page whose response never arrived after this long, so a dropped
/// response cannot wedge scroll-back forever.
pub const PENDING_HISTORY_PAGE_RETRY: Duration = Duration::from_secs(30);

/// An older-history page awaiting its response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingHistoryPage {
    /// Correlation id; only an EXACT match may apply a page.
    pub request_id: String,
    /// Cursor the request was issued for.
    pub before: String,
    /// When the request was issued, for the staleness retry.
    pub requested_at: Instant,
}

/// A decision to fetch one older page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPageRequest {
    pub request_id: String,
    pub before: String,
}

/// How a reconciled page joins the already-loaded transcript prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixPlan {
    /// Prepend the page ahead of everything currently loaded.
    Prepend,
    /// Replace the whole loaded backfill prefix of this length with the page.
    ReplacePrefix(usize),
}

/// Cursor/latch facts carried by a reconciled history payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageFacts {
    pub before: Option<String>,
    pub has_more_before: bool,
    pub trimmed: bool,
    pub page_len: usize,
    /// True when this page is this session's OWN older page (it grows the
    /// loaded prefix); false for a fresh snapshot (it replaces the prefix).
    pub extend_prefix: bool,
}

/// History paging state for a single session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoryPaging {
    /// Whether a COMPLETE (untrimmed) backfill was applied, guarding
    /// re-delivery. Trimmed busy-connect snapshots must not set it (#1050).
    pub backfilled: bool,
    /// Total loaded backfill prefix length while the backfill is still partial.
    pub partial_prefix_len: Option<usize>,
    pub before_cursor: Option<String>,
    pub has_more_before: bool,
    pub page_seq: u64,
    pub pending_page: Option<PendingHistoryPage>,
}

impl HistoryPaging {
    /// Forget any in-flight page (its response can no longer be applied).
    pub fn clear_pending_page(&mut self) {
        self.pending_page = None;
    }

    /// Whether `id` is EXACTLY the in-flight page request. Prefix matches and
    /// absent ids are rejected, so foreign or broadcast `history-page-*`
    /// responses can never prepend history.
    pub fn is_pending_page(&self, id: Option<&str>) -> bool {
        match (id, self.pending_page.as_ref()) {
            (Some(id), Some(pending)) => id == pending.request_id,
            _ => false,
        }
    }

    /// Roll back the in-flight page when its command failed to enqueue, so the
    /// same cursor can be requested again. Ignores unrelated ids.
    pub fn rollback_pending_page(&mut self, id: &str) {
        if self.is_pending_page(Some(id)) {
            self.pending_page = None;
        }
    }

    /// Drop all paging state: the server-side conversation was swapped or
    /// truncated (resume, rewind, clear history), so cursors and in-flight
    /// requests refer to messages that no longer exist.
    pub fn reset(&mut self) {
        self.pending_page = None;
        self.before_cursor = None;
        self.has_more_before = false;
        self.partial_prefix_len = None;
        self.backfilled = false;
    }

    /// Reopen backfill for a transcript that was replaced wholesale, keeping any
    /// cursors the replacement payload published.
    pub fn reopen_backfill(&mut self) {
        self.backfilled = false;
        self.partial_prefix_len = None;
    }

    /// Decide whether to fetch one older page.
    ///
    /// `at_oldest_loaded` is the only view fact this policy needs. A same-cursor
    /// request already in flight suppresses a duplicate until it goes stale
    /// (`PENDING_HISTORY_PAGE_RETRY`), after which it is re-issued.
    pub fn next_page_request(
        &mut self,
        at_oldest_loaded: bool,
        now: Instant,
        mint_id: impl FnOnce(u64) -> String,
    ) -> Option<HistoryPageRequest> {
        if !self.has_more_before || !at_oldest_loaded {
            return None;
        }
        let before = self.before_cursor.clone()?;
        if let Some(pending) = self.pending_page.as_ref()
            && pending.before == before
            && now.saturating_duration_since(pending.requested_at) < PENDING_HISTORY_PAGE_RETRY
        {
            return None;
        }
        self.page_seq = self.page_seq.wrapping_add(1);
        let request_id = mint_id(self.page_seq);
        self.pending_page = Some(PendingHistoryPage {
            request_id: request_id.clone(),
            before: before.clone(),
            requested_at: now,
        });
        Some(HistoryPageRequest { request_id, before })
    }

    /// Apply a reconciled page's cursors and decide how it joins the loaded
    /// prefix.
    ///
    /// Cursors are published BEFORE the caller's empty-page early return, so an
    /// empty page still leaves paging reachable. Returns `None` when the page
    /// carries no entries — an empty or fully filtered page must never latch
    /// the backfill guard.
    pub fn reconcile(&mut self, facts: &PageFacts) -> Option<PrefixPlan> {
        self.pending_page = None;
        self.before_cursor = facts.before.clone();
        self.has_more_before = facts.has_more_before;
        if facts.page_len == 0 {
            return None;
        }

        // `partial_prefix_len` tracks the TOTAL loaded backfill prefix, so a
        // later snapshot replaces every previously loaded page — never just the
        // most recent one (which would duplicate the newest slice and leave an
        // interior gap).
        let (plan, loaded_prefix) = if facts.extend_prefix {
            (
                PrefixPlan::Prepend,
                self.partial_prefix_len.unwrap_or(0) + facts.page_len,
            )
        } else if let Some(partial_len) = self.partial_prefix_len {
            (PrefixPlan::ReplacePrefix(partial_len), facts.page_len)
        } else {
            (PrefixPlan::Prepend, facts.page_len)
        };

        if facts.trimmed || facts.has_more_before {
            self.partial_prefix_len = Some(loaded_prefix);
            self.backfilled = false;
        } else {
            self.partial_prefix_len = None;
            self.backfilled = true;
        }
        Some(plan)
    }
}

#[cfg(test)]
#[path = "history_paging_tests.rs"]
mod history_paging_tests;
