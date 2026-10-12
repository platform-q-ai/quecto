//! Board op telemetry (epic #2481, T1): what one project board request
//! did, as the event log's `board_op` record holds it. Pure values, built
//! the way the swarm board's `swarm_op` is (`domain::swarm::telemetry`).
//! T1 holds the record's skeleton; later slices add their fields (T3 the
//! front ends', T4 the use cases', T5 the store's, T6 the index's).
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
    /// The whole request, from its arrival to its answer.
    pub duration_us: u64,
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
