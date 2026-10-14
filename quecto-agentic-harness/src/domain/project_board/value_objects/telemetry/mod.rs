//! Board op telemetry (epic #2481, T1): what one project board request
//! did, as the event log's `board_op` record holds it. Pure values, built
//! the way the swarm board's `swarm_op` is (`domain::swarm::telemetry`).
//! T1 holds the record's skeleton, T3 the front ends' fields and T4 the
//! use cases'; later slices add theirs (T5 the store's, T6 the index's).
//! S4 adds a start's, its claim renewals' and its fold-back's records,
//! under the same rules.
//!
//! **No free text is ever a field.** Enforced: a field is a kind enum, a
//! count, or a leaf from `leaves` ([`StaticKind`], a known kind or
//! `other`; [`TaskRef`], a slug redacted and capped) or `code_ref`
//! ([`CommitId`], a full commit id), and an architecture test holds this
//! file, `leaves.rs` and `commit_id.rs` to whole-file snapshots and the
//! record's fields to those types. Convention, for the slices that fill a
//! record: titles, descriptions, messages, problem or reason text, paths,
//! emails, names, remote URLs, tokens and stderr are never recorded, and
//! `head` comes only from an id git produced.
//!
//! **Write-only.** Enforced: a record is `Serialize` only, and
//! `AuditEvent::BoardOp` is never deserialized; a reader (T8) brings its
//! own reader-side type.
//!
//! **Off means off** (owner condition 2026-10-12: "all logging can be
//! turned off in config"). Enforced: a record reaches a log only through
//! an `application::project_board::telemetry::BoardRecording`, which
//! exists only while its `RecordingSwitch` is on, and only composition
//! builds either. Convention, held by T2's contract tests: composition
//! builds the switch from `telemetry.event_log.enabled`, so while it is
//! off nothing is timed and nothing is written, to the event log, to
//! `tracing` or to the CLI's daily `board-cli_<date>.jsonl`.
//!
//! **Kinds are additive:** one is added, never renamed or reused (the
//! frozen lists in this module's tests and `KNOWN_KINDS` hold the names),
//! and a reader of the log must accept a kind it does not know.
use serde::Serialize;

use super::code_ref::CommitId;
use super::vocabulary::RunOutcome;
pub use leaves::{StaticKind, TaskRef};

mod leaves;

/// Declares a kind enum: snake_case on the wire, with `ALL` and `as_str`.
macro_rules! kinds {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        pub enum $name { $(#[serde(rename = $text)] $variant),+ }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }
    };
}

kinds!(
    /// The op a request named: one per name of the board's `OPS` table,
    /// or `unknown` for a request that names none.
    OpKind {
        Bootstrap => "bootstrap", List => "list", ListReady => "list_ready",
        ListTakeable => "list_takeable", Show => "show", Create => "create",
        UpdateDraft => "update_draft", Move => "move", Renew => "renew",
        TakeOver => "take_over", LinkPr => "link_pr", UnlinkPr => "unlink_pr",
        Complete => "complete", ClearSetAside => "clear_set_aside", Unknown => "unknown",
        ProposePlan => "propose_plan", EditPlan => "edit_plan",
    }
);

impl OpKind {
    /// The kind of the op named `name`; `unknown` for any other text.
    pub fn named(name: &str) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| *kind != Self::Unknown && kind.as_str() == name)
            .unwrap_or(Self::Unknown)
    }
}

kinds!(FrontEnd { Tool => "tool", Cli => "cli" });
kinds!(Client { Agent => "agent", PocketRelay => "pocket_relay", Cli => "cli", Other => "other" });
kinds!(Access { Read => "read", Write => "write" });
kinds!(
    /// How a request's field was wrong.
    FaultKind {
        Missing => "missing", Unexpected => "unexpected", WrongType => "wrong_type",
        InvalidValue => "invalid_value", TooLarge => "too_large", NotJson => "not_json",
        DuplicateKey => "duplicate_key",
    }
);

kinds!(
    /// What came of completing a task after a change that may complete it
    /// (T4): every linked pull request merged, some not merged yet, or not
    /// judged (the host, the store or a rule failed it).
    CompletionKind { Done => "done", Pending => "pending", Failed => "failed" }
);
kinds!(
    /// A kind of change the board's normaliser made to a text field as it
    /// entered the board (T4), never the text: a CRLF, a lone CR or
    /// another line break made one LF; a chat space or a tab made a plain
    /// space; blank lines gone from a text's edges; trailing spaces gone
    /// from a line; spacing gone from a one-line field's edges.
    TextChange {
        Crlf => "crlf", LoneCr => "lone_cr", LineBreak => "line_break",
        ChatSpace => "chat_space", Tab => "tab", EdgeBlankLines => "edge_blank_lines",
        TrailingSpace => "trailing_space", EdgeSpace => "edge_space",
    }
);

/// How the request ended: `ok`, or refused with its `ErrorCode` and, for a
/// rule, the rule's code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum OpOutcome {
    Ok,
    Refused {
        code: StaticKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        rule: Option<StaticKind>,
    },
}

/// Which field of a refused request was wrong, and how; never the problem
/// text. `field` is a schema field of `OPS`, or a pseudo-field: `op` (the
/// request's op was missing or unknown), `request` (the request as a
/// whole: not JSON, not an object), or [`StaticKind::OTHER`] (a key no
/// schema names, counted in `unexpected_count`, never named).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestFault {
    pub field: StaticKind,
    pub fault: FaultKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unexpected_count: Option<u64>,
}

/// One board request, as the event log's `board_op` record holds it; its
/// envelope's `turn` is `null`. Each optional field is left out when it
/// does not apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardOpRecord {
    pub op: OpKind,
    pub front_end: FrontEnd,
    pub client: Client,
    pub access: Access,
    #[serde(flatten)]
    pub outcome: OpOutcome,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub fault: Option<RequestFault>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<CommitId>,
    /// A listing's counts (T3), from the answer its front end gave.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub listing: Option<ListingCounts>,
    /// The plan versions and runs an answered `show` gave (#2481 M4a).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plans: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<u64>,
    /// The plan version a `propose_plan` wrote, or an `edit_plan` named
    /// (#2481 M4b).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_version: Option<u64>,
    /// The task items and team roles of the plan a `propose_plan` or
    /// `edit_plan` wrote (#2481 M4b).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<u64>,
    /// The task's status as a change last read it (T4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_from: Option<StaticKind>,
    /// The status a change moved to: a move's target, made or refused;
    /// otherwise the status of the task it wrote (T4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_to: Option<StaticKind>,
    /// The claim a change found on its task (T4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<ClaimFacts>,
    /// The attempts a write began, and how many of them lost the race to
    /// another writer (T4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lost: Option<u64>,
    /// How long a write paused between its attempts, when it did (T4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause_ms: Option<u64>,
    /// What came of completing the task (T4).
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionFacts>,
    /// What the normaliser changed in the text fields a request gave (T4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalised: Option<Normalised>,
    /// The call was dropped (its turn interrupted) before it was answered:
    /// written only when it was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    /// How long composition took to build the board's operations, when
    /// the request got that far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_us: Option<u64>,
    /// The answer's size as its front end sent it.
    pub result_bytes: u64,
    /// The whole request, from its arrival to its answer.
    pub duration_us: u64,
}

/// What a listing's answer held (T3): the tasks it listed and showed, and
/// how many it left out; the files it showed set aside, and how many of
/// those it left out. The CLI leaves nothing out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ListingCounts {
    pub listed: u64,
    pub shown: u64,
    pub truncated_more: u64,
    pub set_aside: u64,
    pub set_aside_more: u64,
}

/// A task's claim as a change found it (T4), never its holder: how long
/// it had been held and had left, in seconds (`left_s` below zero once it
/// lapsed; `held_s` below zero when the writer's clock reads before the
/// claim began), and whether the writer holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ClaimFacts {
    pub held_s: i64,
    pub left_s: i64,
    pub holder_is_writer: bool,
}

/// What came of completing a task (T4): its kind; for `failed`, the error
/// code of the refusal that failed it and, for a rule's, the rule's code;
/// the pull requests the task links, and those not merged yet; and, for a
/// completion run after a change, its own write's attempts, lost races
/// and pause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompletionFacts {
    pub completion: CompletionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_code: Option<StaticKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_rule: Option<StaticKind>,
    pub prs_linked: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prs_pending: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_attempts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_lost: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_pause_ms: Option<u64>,
}

/// What the normaliser changed in each text field a request gave (T4):
/// left out for a field the request did not give, and empty for one it
/// kept as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Normalised {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<Vec<TextChange>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Vec<TextChange>>,
    /// A plan's approach (#2481 M4b).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach: Option<Vec<TextChange>>,
    /// A plan's task items: every change made to any of their titles
    /// and criteria (#2481 M4b).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<TextChange>>,
}

kinds!(
    /// Whether a start took the task's claim (it was ready) or found it
    /// held by the starter (S4).
    StartClaim { Fresh => "fresh", Held => "held" }
);

kinds!(
    /// How a folded run ended (S4): a run outcome, by its own name.
    RunOutcomeKind { Succeeded => "succeeded", Failed => "failed", Stopped => "stopped", Lost => "lost" }
);

impl RunOutcomeKind {
    /// The kind of `outcome`.
    pub fn of(outcome: RunOutcome) -> Self {
        match outcome {
            RunOutcome::Succeeded => Self::Succeeded,
            RunOutcome::Failed => Self::Failed,
            RunOutcome::Stopped => Self::Stopped,
            RunOutcome::Lost => Self::Lost,
        }
    }
}

/// One start of a task (S4, #2479), as the event log's `board_start`
/// record holds it: what was started, how large a team, whether the claim
/// was taken, and how long the coordinator's launch took. Each optional
/// field is left out when the start did not get that far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardStartRecord {
    pub task: TaskRef,
    pub client: Client,
    #[serde(flatten)]
    pub outcome: OpOutcome,
    /// The run's number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_version: Option<u64>,
    /// The member limit, the coordinator included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<u64>,
    /// The task items the brief seeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<u64>,
    /// The worker roles whose models were not applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_applied: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<StartClaim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_us: Option<u64>,
    pub duration_us: u64,
}

/// A started run's claim renewals (S4): consecutive successful renewals
/// folded into one record, with the least time the claim had left before
/// one; a failed renewal is a record of its own, refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimRenewalsRecord {
    pub task: TaskRef,
    pub run: u64,
    #[serde(flatten)]
    pub outcome: OpOutcome,
    pub renewals: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_left_s: Option<i64>,
}

/// A run folded back (S4): how it ended (`run_outcome`), where the plan's task items stood, the plan items the swarm
/// never seeded (`unmatched`) and the swarm items no plan item matched
/// (`extra`), the evidence lines kept and dropped, its cost, and its
/// write's attempts, lost races and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoldBackRecord {
    pub task: TaskRef,
    pub run: u64,
    #[serde(flatten)]
    pub outcome: OpOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_outcome: Option<RunOutcomeKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<ItemCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmatched: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_dropped: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lost: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub duration_us: u64,
}

/// A run's task items by the state each stood in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ItemCounts {
    pub todo: u64,
    pub doing: u64,
    pub review: u64,
    pub done: u64,
    pub blocked: u64,
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
