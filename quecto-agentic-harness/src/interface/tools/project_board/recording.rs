//! A board call's `board_op` record (epic #2481 T3): what one request did,
//! as its front end answered it. Each front end starts a [`BoardCall`]
//! with the recording composition gave it (none while the event log is
//! off), answers through it, so an answer and its record are made in one
//! place and cannot drift apart, and finishes it with the answer it sent:
//! one record per call.
//!
//! Off means off: a call with no recording keeps no clock and notes
//! nothing. A record holds only kinds the code spells (an op of `OPS`, an
//! [`ErrorCode`](super::answer::ErrorCode), a rule's code, a schema
//! field), counts, a task's redacted slug, a pull request's number and a
//! commit id git produced; never the request's text, nor any of the
//! answer's.
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::answer::{self, FrontEndRefusal, rule_code};
use super::request::{RequestError, schema_field};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::operations::{BoardOutcome, BoardRequest};
use crate::application::project_board::telemetry::{
    BoardRecording, access_of, op_kind, pr_of, task_of,
};
use crate::domain::project_board::value_objects::code_ref::CommitId;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::telemetry::{
    Access, BoardOpRecord, Client, FaultKind, FrontEnd, ListingCounts, OpKind, OpOutcome,
    RequestFault, StaticKind, TaskRef,
};

/// One front end call on the board, and its record while it is recorded.
pub struct BoardCall {
    open: Option<Open>,
}

/// A recorded call: where its record goes, when it started, and what is
/// known of it so far.
struct Open {
    recording: Arc<BoardRecording>,
    started: Instant,
    record: BoardOpRecord,
}

impl BoardCall {
    /// A call from `front_end` for `client`, recorded in `recording` when
    /// there is one; its clock starts now.
    pub fn start(
        recording: Option<Arc<BoardRecording>>,
        front_end: FrontEnd,
        client: Client,
    ) -> Self {
        let open = recording.map(|recording| Open {
            recording,
            started: Instant::now(),
            record: BoardOpRecord {
                op: OpKind::Unknown,
                front_end,
                client,
                access: Access::Read,
                outcome: OpOutcome::Ok,
                fault: None,
                task: None,
                pr: None,
                head: None,
                listing: None,
                cancelled: false,
                build_us: None,
                result_bytes: 0,
                duration_us: 0,
            },
        });
        Self { open }
    }

    /// A call recorded nowhere.
    pub fn unrecorded() -> Self {
        Self { open: None }
    }

    fn note(&mut self, note: impl FnOnce(&mut BoardOpRecord)) {
        if let Some(open) = &mut self.open {
            note(&mut open.record);
        }
    }

    /// The request names the op `op` (`None`: none, or none of `OPS`).
    pub fn named(&mut self, op: Option<&str>) {
        self.note(|record| {
            record.op = op.map_or(OpKind::Unknown, OpKind::named);
            record.access = access_of(record.op);
        });
    }

    /// The request read as `request`: its op, access, task and pull
    /// request.
    pub fn parsed(&mut self, request: &BoardRequest) {
        self.note(|record| {
            record.op = op_kind(request);
            record.access = access_of(record.op);
            record.task = task_of(request).map(TaskRef::of);
            record.pr = pr_of(request);
        });
    }

    /// What `build` builds (the board's operations), timed while recorded.
    pub fn built<T>(&mut self, build: impl FnOnce() -> T) -> T {
        let started = self.open.as_ref().map(|_| Instant::now());
        let built = build();
        if let Some(started) = started {
            self.note(|record| record.build_us = Some(micros(started.elapsed())));
        }
        built
    }

    /// The answer refusing the request to `op` before the board was read.
    pub fn refuse_request(&mut self, op: Option<&str>, error: &RequestError) -> Value {
        self.note(|record| {
            record.outcome = refused(answer::ErrorCode::InvalidRequest.as_str(), None);
            record.fault = Some(error.fault.clone());
        });
        answer::request_refused(op, error)
    }

    /// The answer of a front end's own refusal of `op`, with `reason`.
    pub fn refuse_as_front_end(
        &mut self,
        op: Option<&str>,
        refusal: FrontEndRefusal,
        reason: &str,
    ) -> Value {
        self.note(|record| record.outcome = refused(refusal.code().as_str(), None));
        answer::reason_refused(op, refusal, reason)
    }

    /// The answer of the board's refusal of `op`.
    pub fn refuse_as_board(&mut self, op: &str, error: &ProjectBoardError) -> Value {
        self.note(|record| {
            // Only a rule's refusal has a rule's code.
            let rule = match error {
                ProjectBoardError::Rule(rule) => Some(rule_code(rule)),
                _ => None,
            };
            record.outcome = refused(answer::error_code(error).as_str(), rule);
            record.fault = schema_fault(error);
        });
        answer::board_refused(op, error)
    }

    /// The answer to `op`, which came to `outcome`; `clock` as
    /// [`answer::outcome`] takes it. The head it names is one git produced.
    pub fn answer(&mut self, op: &str, outcome: &BoardOutcome, clock: Option<String>) -> Value {
        self.note(|record| {
            let head = match outcome {
                BoardOutcome::Bootstrapped { head, .. }
                | BoardOutcome::Shown { head, .. }
                | BoardOutcome::Cleared { head } => Some(head),
                BoardOutcome::Listing(listing) => Some(&listing.head),
                BoardOutcome::Changed { .. } => None,
            };
            record.outcome = OpOutcome::Ok;
            record.head = head.and_then(|head| CommitId::parse(&head.0).ok());
        });
        answer::outcome(op, outcome, clock)
    }

    /// Writes the call's record, once its front end sent `answer`:
    /// `cancelled` when the call was dropped before it came back.
    pub fn finish(self, answer: &Value, cancelled: bool) {
        let Some(Open {
            recording,
            started,
            mut record,
        }) = self.open
        else {
            return;
        };
        let listing = matches!(
            record.op,
            OpKind::List | OpKind::ListReady | OpKind::ListTakeable
        ) && record.outcome == OpOutcome::Ok;
        record.listing = listing.then(|| listing_counts(answer)).flatten();
        record.cancelled = cancelled;
        record.result_bytes = u64::try_from(answer.to_string().len()).unwrap_or(u64::MAX);
        record.duration_us = micros(started.elapsed());
        recording.record(record);
    }
}

/// Refused with `code`, and `rule`'s code for a rule: each an entry of the
/// code's own tables, never text from outside.
fn refused(code: &'static str, rule: Option<&'static str>) -> OpOutcome {
    OpOutcome::Refused {
        code: StaticKind::new(code),
        rule: rule.map(StaticKind::new),
    }
}

/// Which schema field a refused task's write named: the task's
/// (`invalid_task`) or a rule's (`invalid`).
fn schema_fault(error: &ProjectBoardError) -> Option<RequestFault> {
    // Only these two refusals name a schema field.
    let schema = match error {
        ProjectBoardError::Invalid(schema)
        | ProjectBoardError::Rule(BoardRuleError::Invalid(schema)) => schema,
        _ => return None,
    };
    Some(RequestFault {
        field: schema_field(&schema.field),
        fault: FaultKind::InvalidValue,
        unexpected_count: None,
    })
}

/// A listing's counts from its answer: the tasks and set-aside files it
/// shows, and the `truncated.more` and `quarantined_more` it left out.
fn listing_counts(answer: &Value) -> Option<ListingCounts> {
    let count = |items: usize| u64::try_from(items).unwrap_or(u64::MAX);
    let shown = count(answer.get("tasks")?.as_array()?.len());
    let truncated_more = answer["truncated"]["more"].as_u64().unwrap_or(0);
    let files = answer["quarantined"].as_array().map_or(0, Vec::len);
    let set_aside_more = answer["quarantined_more"].as_u64().unwrap_or(0);
    Some(ListingCounts {
        listed: shown.saturating_add(truncated_more),
        shown,
        truncated_more,
        set_aside: count(files).saturating_add(set_aside_more),
        set_aside_more,
    })
}

fn micros(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
