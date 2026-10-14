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
use crate::application::project_board::operations::{BoardOutcome, BoardRequest, Completion};
use crate::application::project_board::telemetry::{
    BoardRecording, OpFacts, access_of, op_kind, pr_of, task_of,
};
use crate::application::project_board::use_cases::RunBoardOperation;
use crate::domain::project_board::value_objects::code_ref::CommitId;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::telemetry::{
    Access, BoardOpRecord, Client, CompletionKind, FaultKind, FrontEnd, ListingCounts, OpKind,
    OpOutcome, RequestFault, StaticKind, TaskRef,
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
        let arrival = recording.map(|recording| (recording, Instant::now()));
        Self::arrived(arrival, front_end, client)
    }

    /// A call from `front_end` for `client` that arrived when `arrival`
    /// says, with the recording it is recorded in (`None`: recorded
    /// nowhere, so its arrival was never timed either).
    pub fn arrived(
        arrival: Option<(Arc<BoardRecording>, Instant)>,
        front_end: FrontEnd,
        client: Client,
    ) -> Self {
        let open = arrival.map(|(recording, started)| Open {
            recording,
            started,
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
                plans: None,
                runs: None,
                plan_version: None,
                items: None,
                roles: None,
                plan_state_from: None,
                plan_state_to: None,
                status_from: None,
                status_to: None,
                claim: None,
                attempts: None,
                lost: None,
                pause_ms: None,
                completion: None,
                normalised: None,
                cancelled: false,
                build_us: None,
                result_bytes: 0,
                duration_us: 0,
            },
        });
        Self { open }
    }

    /// A call recorded nowhere: the tests' (every front end starts its
    /// calls with the recording composition gave it).
    #[cfg(test)]
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

    /// What `operations` make of `request`, with what its use cases saw
    /// (T4) noted while recorded; unrecorded, nothing is noted or timed.
    ///
    /// # Errors
    /// As [`RunBoardOperation::execute`].
    pub fn executed(
        &mut self,
        operations: &RunBoardOperation,
        request: &BoardRequest,
    ) -> Result<BoardOutcome, ProjectBoardError> {
        let Some(open) = &mut self.open else {
            return operations.execute(request);
        };
        let mut facts = OpFacts::default();
        let result = operations.execute_noted(request, Some(&mut facts));
        facts.fill(&mut open.record);
        result
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
            let (rule, schema) = refusal_parts(error);
            record.outcome = refused(answer::error_code(error).as_str(), rule);
            record.fault = schema.map(|schema| schema_fault(error, schema));
        });
        answer::board_refused(op, error)
    }

    /// The answer to `op`, which came to `outcome`; `clock` as
    /// [`answer::outcome`] takes it. The head it names is one git produced.
    pub fn answer(&mut self, op: &str, outcome: &BoardOutcome, clock: Option<String>) -> Value {
        self.note(|record| {
            let head = match outcome {
                BoardOutcome::Bootstrapped { head, .. } | BoardOutcome::Cleared { head } => {
                    Some(head)
                }
                BoardOutcome::Shown(shown, _) => Some(&shown.head),
                BoardOutcome::Listing(listing) => Some(&listing.head),
                BoardOutcome::Changed { .. }
                | BoardOutcome::Planned(_)
                | BoardOutcome::Accepted(_) => None,
            };
            record.outcome = OpOutcome::Ok;
            record.head = head.and_then(|head| CommitId::parse(&head.0).ok());
            // A failed completion's code and rule, as the answer spells them.
            if let (
                BoardOutcome::Changed {
                    completion: Some(Completion::Failed(error)),
                    ..
                },
                Some(completion),
            ) = (outcome, &mut record.completion)
            {
                debug_assert_eq!(completion.completion, CompletionKind::Failed);
                completion.completion_code =
                    Some(StaticKind::new(answer::error_code(error).as_str()));
                completion.completion_rule = refusal_parts(error).0.map(StaticKind::new);
            }
        });
        answer::outcome(op, outcome, clock)
    }

    /// Writes the call's record, once its front end sent `answer`:
    /// `cancelled` when the call was dropped before it came back.
    pub fn finish(mut self, answer: &Value, cancelled: bool) {
        let Some(Open {
            recording,
            started,
            mut record,
        }) = self.open.take()
        else {
            return;
        };
        let (noted, agreed) = (record.outcome.clone(), agrees(&record.outcome, answer));
        record.outcome = answered_outcome(noted.clone(), answer);
        let listing = matches!(
            record.op,
            OpKind::List | OpKind::ListReady | OpKind::ListTakeable
        ) && record.outcome == OpOutcome::Ok;
        record.listing = listing.then(|| listing_counts(answer)).flatten();
        record.cancelled = cancelled;
        record.result_bytes = u64::try_from(answer.to_string().len()).unwrap_or(u64::MAX);
        record.duration_us = micros(started.elapsed());
        recording.record(record);
        // Written first; the message holds the answer's `ok` and the
        // noted kinds only, never the answer's text.
        debug_assert!(
            agreed,
            "the answer's ok {:?} disagrees with its note {noted:?}",
            answer.get("ok")
        );
    }
}

impl Drop for BoardCall {
    /// A recorded call that ends without being finished is recorded all
    /// the same, with no answer to size or count: refused as `internal`
    /// when its work panicked (the call unwinds), `cancelled` when it was
    /// otherwise dropped unanswered.
    fn drop(&mut self) {
        if let Some(Open {
            recording,
            started,
            mut record,
        }) = self.open.take()
        {
            match std::thread::panicking() {
                true => record.outcome = refused("internal", None),
                false => record.cancelled = true,
            }
            record.duration_us = micros(started.elapsed());
            recording.record(record);
        }
    }
}

/// Whether `answer` says what its note `noted` says: ok, or refused.
fn agrees(noted: &OpOutcome, answer: &Value) -> bool {
    let answered_ok = answer.get("ok") == Some(&Value::Bool(true));
    answered_ok == (*noted == OpOutcome::Ok)
}

/// The outcome a call's record holds, once its front end sent `answer`:
/// the one noted, when the answer [`agrees`] (it always should: an answer
/// and its note are made by the same call, and a debug build asserts it).
/// Otherwise the answer is the truth: `ok`, or refused with the answer's
/// code as a known kind; never a panic on a user's call.
fn answered_outcome(noted: OpOutcome, answer: &Value) -> OpOutcome {
    let answered_ok = answer.get("ok") == Some(&Value::Bool(true));
    match (agrees(&noted, answer), answered_ok) {
        (true, _) => noted,
        (false, true) => OpOutcome::Ok,
        (false, false) => OpOutcome::Refused {
            code: answer["error"]["code"]
                .as_str()
                .map_or(StaticKind::OTHER, StaticKind::new),
            rule: None,
        },
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

/// What a board's refusal names besides its code: a rule's code, and the
/// schema error whose field was wrong (a task's, `invalid_task`, or a
/// rule's, `invalid`). Exhaustive, so a new refusal does not compile until
/// it says which it names.
fn refusal_parts(error: &ProjectBoardError) -> (Option<&'static str>, Option<&SchemaError>) {
    use ProjectBoardError as E;
    match error {
        E::Rule(rule) => (Some(rule_code(rule)), rule_schema(rule)),
        E::Invalid(schema) => (None, Some(schema)),
        E::Store(_)
        | E::NotFound(_)
        | E::Exists(_)
        | E::IdInUse { .. }
        | E::Clock(_)
        | E::Quarantined { .. }
        | E::Duplicate { .. }
        | E::SetAsideFile { .. }
        | E::UnknownReference(_)
        | E::ClosesCycle { .. }
        | E::InvalidWriter(_)
        | E::PrStateUnknown(_)
        | E::PullRequests(_)
        | E::TaskAhead { .. }
        | E::ClockOutOfRange(_)
        | E::ClockAhead { .. }
        | E::Contended { .. }
        | E::Refused(_)
        | E::NotSetAside { .. }
        | E::NoPullRequestHost(_)
        | E::Stale { .. } => (None, None),
    }
}

/// The fault a board's schema refusal records: a plan's text as a whole
/// past the most a plan holds (`invalid_plan`'s `text`, #2481 M4b) is the
/// request as a whole, too large, as it was recorded when the request
/// judged it; any other field is its schema field, of a value refused.
fn schema_fault(error: &ProjectBoardError, schema: &SchemaError) -> RequestFault {
    let plan_text = matches!(
        error,
        ProjectBoardError::Rule(BoardRuleError::InvalidPlan(_))
    ) && schema.field == "text";
    let (field, fault) = match plan_text {
        true => (StaticKind::new("request"), FaultKind::TooLarge),
        false => (schema_field(&schema.field), FaultKind::InvalidValue),
    };
    RequestFault {
        field,
        fault,
        unexpected_count: None,
    }
}

/// The schema error a rule's refusal names: only `invalid` and
/// `invalid_plan` name one.
/// Exhaustive, as [`refusal_parts`].
fn rule_schema(rule: &BoardRuleError) -> Option<&SchemaError> {
    use BoardRuleError as R;
    match rule {
        R::Invalid(schema) | R::InvalidPlan(schema) => Some(schema),
        R::NotAllowed { .. }
        | R::NotHolder
        | R::NoClaim
        | R::ClaimLive
        | R::ClaimExpired
        | R::RunOpen(..)
        | R::RunEnded(..)
        | R::PlanNotDraft(..)
        | R::NotThisTasks
        | R::NotThisChange
        | R::TaskFinished(..)
        | R::UnknownPlanVersion(..)
        | R::PlanChanged { .. }
        | R::PlansInconsistent(..)
        | R::PlanAhead { .. }
        | R::RunsInconsistent(..)
        | R::PlanNotAccepted(..)
        | R::NoAcceptedPlan
        | R::ChangeAlreadyLinked { .. }
        | R::NoSuchChange(..)
        | R::ChangeAhead { .. }
        | R::DiscussionClosed(..)
        | R::DiscussionLinked { .. }
        | R::DiscussionAhead { .. }
        | R::MessageAhead { .. }
        | R::DiscussionFull { .. }
        | R::ReviewAhead { .. }
        | R::ChangesLinked(..)
        | R::PrUnread(..)
        | R::PrNotOpen(..)
        | R::PrHeadMismatch(..)
        | R::ChangeUnlinked(..)
        | R::ChangeMerged(..)
        | R::NoChanges
        | R::ChangesUnmerged(..)
        | R::ChangesInconsistent(..)
        | R::NotTakeable(..)
        | R::AlreadyHolder
        | R::NoPrs
        | R::PrsUnmerged(..)
        | R::PrsLinked(..)
        | R::PrNotLinked(..)
        | R::PrAlreadyLinked(..)
        | R::NotDraft(..)
        | R::NoEdit
        | R::Superseded { .. }
        | R::IssueShared { .. }
        | R::IssueIsPr { .. }
        | R::OwnPrReferenced { .. } => None,
    }
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

/// The tests' recordings and outside text, shared with the front ends'
/// tests.
#[cfg(test)]
#[path = "recording_tests.rs"]
pub(crate) mod tests;
