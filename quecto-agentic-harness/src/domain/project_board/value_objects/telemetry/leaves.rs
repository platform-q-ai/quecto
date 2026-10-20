//! The record's guarded leaves (epic #2481 T1): the only values in a
//! `board_op` record that hold text. Enforced: each one's field is private
//! to this module, so no other module can build one but through the
//! constructors here; and an architecture test holds this whole file, doc
//! comments aside, to a snapshot, so any change to it is a reviewed one.
use serde::Serialize;

use crate::domain::project_board::value_objects::slug::{MAX_SLUG_BYTES, Slug};
use crate::domain::shared::redaction::Redacted;

/// The most characters a [`TaskRef`] records (owner decision, 2026-10-12).
const MAX_TASK_REF_CHARS: usize = 64;

// Every slug fits whole, so the cap never cuts a slug short.
const _: () = assert!(MAX_SLUG_BYTES <= MAX_TASK_REF_CHARS);

/// The most bytes a kind may take.
const MAX_KIND_BYTES: usize = 48;

/// Whether `text` is shaped as a kind: 1 to [`MAX_KIND_BYTES`] lowercase
/// ASCII letters, digits and underscores (swarm's `decision_kind`, and
/// bounded), so no sentence, path, address or credential passes as one.
const fn kind_shaped(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if !matches!(bytes[at], b'a'..=b'z' | b'0'..=b'9' | b'_') {
            return false;
        }
        at += 1;
    }
    !bytes.is_empty() && bytes.len() <= MAX_KIND_BYTES
}

/// Every kind a record may hold, frozen (append-only): the op names, the
/// outcomes, front ends, clients, accesses and faults, the task statuses,
/// the board's error codes, `internal` (a call that ended without an
/// answer), rule codes, completions and normaliser changes (T4), S4's
/// start, renewal and fold-back kinds, and the schema fields of `OPS`
/// with the pseudo-fields `op`, `request` and `other`. Any other text is
/// recorded as `other`. A test in `answer_tests.rs` holds every error and
/// rule code the board answers to this list.
const KNOWN_KINDS: &[&str] = &[
    // Ops.
    "bootstrap",
    "list",
    "list_ready",
    "list_takeable",
    "show",
    "create",
    "update_draft",
    "move",
    "renew",
    "take_over",
    "link_pr",
    "unlink_pr",
    "complete",
    "clear_set_aside",
    "unknown",
    // Ops (#2481 M4b).
    "propose_plan",
    "edit_plan",
    // Ops and plan states (#2481 M4c); `draft` and `superseded` are
    // listed already.
    "accept_plan",
    "discard_plan",
    "accepted",
    "discarded",
    // An acceptance's pin on the plan it saw accepted (#2481 M4c review
    // round 1): its field and its refusal.
    "expected_accepted",
    "accepted_plan_changed",
    // A start's refusals (S4c): no launcher, a plan that cannot brief a
    // swarm (its own codes are S4's, above), a launch that failed.
    "no_launcher",
    "brief",
    "launch_failed",
    // A start's undo (S4c review round 2): where it left the run.
    "folded",
    "ended_already",
    "open",
    // A run ended by hand (S4c): its op and fields; its outcomes are S4's;
    // its refusal while the run's swarm runs here.
    "run_live",
    "end_run",
    "run",
    "outcome",
    // The start op and its field (S4c).
    "start",
    "plan_version",
    // Outcomes, front ends, clients, accesses, faults.
    "ok",
    "refused",
    "tool",
    "cli",
    "agent",
    "pocket_relay",
    "read",
    "write",
    "missing",
    "unexpected",
    "wrong_type",
    "invalid_value",
    "too_large",
    "not_json",
    "duplicate_key",
    // Task statuses.
    "draft",
    "ready",
    "claimed",
    "in_progress",
    "review",
    "blocked",
    "done",
    "archived",
    // Error codes.
    "invalid_request",
    "no_committer",
    "no_board",
    "clash",
    "invalid_write",
    "store_unavailable",
    "rule",
    "invalid_task",
    "invalid_writer",
    "not_found",
    "exists",
    "clock_unreadable",
    "set_aside",
    "artifact_set_aside",
    "duplicate_artifact",
    "set_aside_file",
    "unknown_reference",
    "closes_cycle",
    "pr_state_unknown",
    "pull_requests_unavailable",
    "task_ahead",
    "clock_out_of_range",
    "clock_ahead",
    "contended",
    "stale",
    "not_set_aside",
    "no_pull_request_host",
    "id_in_use",
    // A call that ended without an answer (T3 review round 1).
    "internal",
    // A board past what a read takes (#2481 S9 review round 1), and one
    // held outside tasks/: a file set aside there naming another kind's
    // key holds listings.
    "board_too_large",
    "folder_set_aside",
    // Rule codes.
    "not_allowed",
    "not_holder",
    "no_claim",
    "claim_live",
    "claim_expired",
    "run_open",
    "run_ended",
    "plan_not_draft",
    "not_this_tasks",
    "not_this_change",
    "task_finished",
    "unknown_plan_version",
    "plan_changed",
    "plans_inconsistent",
    "plan_ahead",
    "runs_inconsistent",
    "plan_not_accepted",
    "change_already_linked",
    "no_such_change",
    "no_such_run",
    "change_ahead",
    "discussion_closed",
    "discussion_linked",
    "discussion_ahead",
    "message_ahead",
    "discussion_full",
    "review_ahead",
    "changes_linked",
    "pr_unread",
    "pr_not_open",
    "pr_head_mismatch",
    "change_unlinked",
    "change_merged",
    "no_changes",
    "changes_unmerged",
    "changes_inconsistent",
    "not_takeable",
    "already_holder",
    "no_prs",
    "prs_unmerged",
    "prs_linked",
    "pr_not_linked",
    "pr_already_linked",
    "not_draft",
    "no_edit",
    "superseded",
    // A dependency not done, children still active (#2661).
    "dependency_not_done",
    "children_active",
    "issue_shared",
    "issue_is_pr",
    "own_pr_referenced",
    "invalid",
    "no_accepted_plan",
    // A plan's schema refusal (#2481 M4b).
    "invalid_plan",
    // Completions and normaliser changes (T4).
    "pending",
    "failed",
    "crlf",
    "lone_cr",
    "line_break",
    "chat_space",
    "tab",
    "edge_blank_lines",
    "trailing_space",
    "edge_space",
    // Starts, claim renewals and fold-backs (S4): brief refusals, the team
    // fields spawn cannot carry, the item limits, claim kinds and run
    // outcomes.
    "team_too_large",
    "team_unsupported",
    // Retired, no longer emitted (#2568): a plan's deadline is seconds
    // from Start, so no start refuses one. Kinds are additive.
    "deadline_passed",
    "deadline_too_soon",
    "no_task_items",
    "item_too_large",
    "brief_too_large",
    "coordinator_model",
    "image",
    "turns",
    "members",
    "workers",
    "criteria",
    "fresh",
    "held",
    "succeeded",
    "stopped",
    "lost",
    // A success refused while a task item is not done (#2481 follow-up).
    "items_not_done",
    // Schema fields and pseudo-fields.
    "task",
    "title",
    "kind",
    "description",
    "status",
    "parent",
    "dependents_of",
    "depends_on",
    "expected_updated",
    "to",
    "pr",
    "path",
    "object",
    "repeated",
    // A show's page of its plan (#2481 M4a review round 2).
    "approach_from",
    "items_from",
    // A plan's fields (#2481 M4b); `criteria` is S4's.
    "version",
    "approach",
    "items",
    "team",
    "op",
    "request",
    "other",
];

/// Whether every one of [`KNOWN_KINDS`] is shaped as a kind (checked as
/// the crate builds, below).
const fn all_shaped() -> bool {
    let mut at = 0;
    while at < KNOWN_KINDS.len() {
        if !kind_shaped(KNOWN_KINDS[at]) {
            return false;
        }
        at += 1;
    }
    true
}

const _: () = assert!(all_shaped());

/// A kind the code knows. Enforced: [`Self::new`] holds only an entry of
/// [`KNOWN_KINDS`] (an allowlist frozen in this file's snapshot, each
/// entry shaped as a kind), never the caller's text, and any other text is
/// [`Self::OTHER`]; the field is private to this module, so nothing else
/// builds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct StaticKind(&'static str);

impl StaticKind {
    /// A name that is no kind the code knows: counted, never named.
    pub const OTHER: Self = Self("other");

    /// The known kind `kind` is, or [`Self::OTHER`].
    pub fn new(kind: &str) -> Self {
        KNOWN_KINDS
            .iter()
            .find(|known| **known == kind)
            .map_or(Self::OTHER, |known| Self(known))
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// A task as a record names it: its slug, [`Redacted`], then capped at
/// [`MAX_TASK_REF_CHARS`] characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TaskRef(Redacted);

impl TaskRef {
    pub fn of(slug: &Slug) -> Self {
        let redacted = Redacted::new(slug.as_str());
        let capped: String = redacted.chars().take(MAX_TASK_REF_CHARS).collect();
        Self(Redacted::new(&capped))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[cfg(test)]
#[path = "leaves_tests.rs"]
mod tests;
