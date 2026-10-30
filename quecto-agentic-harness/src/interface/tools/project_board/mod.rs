//! The board operations as their front ends send and answer them (#2477),
//! shared by the `quecto board` CLI and the agent `board` tool (S2b): a
//! request is one JSON object read by [`request`] (the command line builds
//! the same object, [`command_line`]), run by the application's
//! [`BoardOperations`] port, and answered in one stable JSON shape by
//! [`answer`]. One path, so the front ends cannot drift apart: [`run`]
//! takes the request's text, so every front end's request is bounded and
//! read strictly (a key named twice is refused) before anything else.
pub mod answer;
pub mod command_line;
pub mod feed_events;
pub mod fold_back_call;
pub mod json;
pub mod recording;
pub mod request;
pub mod session_recording;
pub mod site;
pub mod tool;

use serde_json::Value;

use crate::application::project_board::dto::ReadRequest;
use crate::application::project_board::operations::{BoardRequest, ListSummaries};
use crate::application::project_board::ports::BoardOperations;
use crate::domain::project_board::value_objects::telemetry::FaultKind;
use recording::BoardCall;
use request::RequestError;

/// The most bytes a request's text may take.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// The request `text` holds: at most [`MAX_REQUEST_BYTES`], JSON naming
/// no key twice.
///
/// # Errors
/// A request too big, not JSON, or naming a key twice.
pub fn read(text: &str) -> Result<Value, RequestError> {
    let refused = |problem: String, fault| RequestError::whole("request", problem, fault);
    if text.len() > MAX_REQUEST_BYTES {
        return Err(refused(
            format!(
                "is {} bytes; a request takes at most {MAX_REQUEST_BYTES}",
                text.len()
            ),
            FaultKind::TooLarge,
        ));
    }
    json::strict(text).map_err(|error| refused(format!("must be JSON: {error}"), error.fault()))
}

/// The answer to the request `text`, recorded nowhere: the tests' way in.
#[cfg(test)]
pub fn run(operations: &dyn BoardOperations, text: &str) -> Value {
    run_recorded(operations, text, &mut BoardCall::unrecorded())
}

/// The answer to the request `text`, noted on `call` (epic #2481 T3).
pub fn run_recorded(operations: &dyn BoardOperations, text: &str, call: &mut BoardCall) -> Value {
    run_recorded_with(operations, text, ListSummaries::Omitted, call)
}

/// [`run_recorded`], a listing summarising its tasks as `summaries` asks
/// (the CLI's `--summary`; the request's JSON names no such field, so the
/// agent tool never asks). Summaries asked of a request that lists
/// nothing are refused as the request's.
pub fn run_recorded_with(
    operations: &dyn BoardOperations,
    text: &str,
    summaries: ListSummaries,
    call: &mut BoardCall,
) -> Value {
    let value = match read(text) {
        Ok(value) => value,
        Err(error) => {
            let op = answer::start_steps::start_in_unread(text);
            call.named(op);
            return call.refuse_request(op, &error);
        }
    };
    let op = request::op_of(&value);
    call.named(op.as_deref());
    let request = match request::parse(&value, request::entry_policy(call.front_end())) {
        Ok(request) => request,
        Err(error) => return call.refuse_request(op.as_deref(), &error),
    };
    call.parsed(&request);
    if let Some(refusal) = summaries_refusal(&request, summaries) {
        return call.refuse_request(op.as_deref(), &refusal);
    }
    let op = op.expect("a parsed request names an op");
    let read = ReadRequest {
        request: &request,
        text,
        front_end: call.front_end(),
    };
    let answer = match call.executed(operations, read, summaries) {
        Ok(answered) => call.answer(&op, &answered.outcome, answered.clock_note),
        Err(error) => {
            let steps = answer::start_steps::refused(&request, &error);
            call.refuse_as_board(&op, &error, steps)
        }
    };
    noted_deprecation(&request, answer)
}

/// What an answer to a request pinned by the deprecated `expected_updated`
/// says of it (#2705 B9), taken for one more release.
pub const DEPRECATED_PIN: &str = "expected_updated is deprecated and will be removed: it no longer \
     tells two edits in one second apart, so it may overwrite a newer edit; pass expected_rev, \
     the rev show gives";

/// `answer`, saying so ([`DEPRECATED_PIN`], as `deprecated`) when
/// `request` was pinned by the deprecated `expected_updated`.
fn noted_deprecation(request: &BoardRequest, mut answer: Value) -> Value {
    let deprecated = request
        .pin()
        .is_some_and(|pin| pin.deprecated_updated().is_some());
    if let (true, Value::Object(fields)) = (deprecated, &mut answer) {
        fields.insert("deprecated".into(), Value::String(DEPRECATED_PIN.into()));
    }
    answer
}

/// Why `summaries` cannot be asked of `request`: only a listing
/// ([`BoardRequest::is_listing`]) is summarised.
pub fn summaries_refusal(request: &BoardRequest, summaries: ListSummaries) -> Option<RequestError> {
    match (summaries, request.is_listing()) {
        (ListSummaries::Omitted, _) | (ListSummaries::Wanted, true) => None,
        (ListSummaries::Wanted, false) => Some(RequestError::of(
            "summary",
            crate::application::project_board::dto::SUMMARIES_LISTINGS_ONLY,
            FaultKind::InvalidValue,
        )),
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plan_ops_tests.rs"]
mod plan_ops_tests;

#[cfg(test)]
#[path = "plan_ops_rev_tests.rs"]
mod plan_ops_rev_tests;

#[cfg(test)]
#[path = "outcome_exception_noop_tests.rs"]
mod outcome_exception_noop_tests;
