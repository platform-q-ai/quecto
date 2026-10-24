//! `POST /v1/boards/{id}/op` (epic #2705 B7): one board request, as its
//! front end read it, run by [`ServeBoardOperation`] and answered through
//! the front ends' own path (`run_recorded_with`), so a remote answer
//! never drifts from a local one. The body is read strictly (a key named
//! twice is refused) within the front ends' request bound; a write needs
//! its `write_id` and `committer`, a read ignores them.
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use crate::application::project_board::dto::{
    BoardExecution, BoardRequest, ListSummaries, OpNoting, ProjectBoardError, ReadRequest, RunEnd,
};
use crate::application::project_board::ports::{
    BoardOperations, OpWrite, ServeError, Transacted, WriteId,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::{
    BoardAccess, Principal, Refused, Scope, ServeBoardOperation,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
use crate::interface::tools::project_board::recording::BoardCall;
use crate::interface::tools::project_board::request::{entry_policy, parse};
use crate::interface::tools::project_board::{read, run_recorded_with};

const BODY_SHAPE: &str = "the body must be {\"request\": {..}, \"front_end\": \
     \"cli\" or \"tool\", \"summary\", \"noted\", and for a write \"write_id\" and \
     \"committer\": {\"name\", \"email\"}}";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OpBody {
    request: Value,
    front_end: String,
    #[serde(default)]
    summary: bool,
    #[serde(default)]
    noted: bool,
    write_id: Option<String>,
    committer: Option<Committer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Committer {
    name: String,
    email: String,
}

impl From<Committer> for Identity {
    fn from(Committer { name, email }: Committer) -> Self {
        Self { name, email }
    }
}

/// Why an op was not answered.
pub(super) enum OpRefusal {
    /// The body is not an op's: why.
    Body(String),
    Access(Refused),
    Serve(ServeError),
}

/// The answer to the op `body` from `principal`.
///
/// # Errors
/// A body not shaped as an op's, an op `principal` may not make, or the
/// server's failure.
pub(super) fn answer_op(
    operations: &ServeBoardOperation,
    principal: &Principal,
    body: &[u8],
) -> Result<String, OpRefusal> {
    let shape = || OpRefusal::Body(BODY_SHAPE.into());
    let text = std::str::from_utf8(body).map_err(|_| shape())?;
    let value = read(text).map_err(|_| shape())?;
    let body: OpBody = serde_json::from_value(value).map_err(|_| shape())?;
    let front_end = FrontEnd::ALL
        .iter()
        .copied()
        .find(|front_end| front_end.as_str() == body.front_end)
        .ok_or_else(shape)?;
    let Value::Object(_) = &body.request else {
        return Err(shape());
    };
    let request = parse(&body.request, entry_policy(front_end)).ok();
    let scope = request.as_ref().map_or(Scope::Read, Scope::of);
    BoardAccess::authorize(principal, scope).map_err(OpRefusal::Access)?;
    let write = match (
        request.as_ref().map(BoardRequest::writes),
        body.write_id,
        body.committer,
    ) {
        (Some(true), Some(id), Some(committer)) => Some(written(principal, &id, committer)?),
        (Some(true), _, _) => return Err(OpRefusal::Serve(ServeError::NoWrite)),
        (Some(false) | None, _, _) => None,
    };
    let summaries = match body.summary {
        true => ListSummaries::Wanted,
        false => ListSummaries::Omitted,
    };
    let noting = match body.noted {
        true => OpNoting::Noted,
        false => OpNoting::Unnoted,
    };
    let text = body.request.to_string();
    let mut answer =
        |board: &dyn BoardOperations| present(board, &text, front_end, summaries, noting);
    operations
        .execute(request.as_ref(), write.as_ref(), &mut answer)
        .map(|served| match served {
            Transacted::Answered(answer) | Transacted::Replayed(answer) => answer,
        })
        .map_err(OpRefusal::Serve)
}

/// The write `id` by `committer`, let in as `principal`.
fn written(principal: &Principal, id: &str, committer: Committer) -> Result<OpWrite, OpRefusal> {
    let write_id = WriteId::parse(id)
        .ok_or_else(|| OpRefusal::Body("write_id must be 8 to 64 of 0-9, a-f and -".into()))?;
    let committer = Identity::from(committer);
    committer
        .validate("committer")
        .map_err(|error| OpRefusal::Body(format!("the committer is invalid: {error}")))?;
    Ok(OpWrite {
        write_id,
        committer,
        principal: principal.recorded(),
    })
}

/// The reply body: the front end's answer to `text` on `board`, and the
/// facts its use cases noted, as far as they travel.
fn present(
    board: &dyn BoardOperations,
    text: &str,
    front_end: FrontEnd,
    summaries: ListSummaries,
    noting: OpNoting,
) -> String {
    let captured = Captured {
        inner: board,
        noting,
        facts: Mutex::new(None),
    };
    let mut call = BoardCall::start(None, front_end, Client::Other);
    let answer = run_recorded_with(&captured, text, summaries, &mut call);
    let facts = captured
        .facts
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let facts = facts.map(|facts| {
        json!({"attempts": facts.attempts, "lost": facts.lost,
               "head": facts.head.map(|head| head.0)})
    });
    json!({ "answer": answer, "facts": facts }).to_string()
}

/// The operations, noted as the client asked, their facts kept.
struct Captured<'a> {
    inner: &'a dyn BoardOperations,
    noting: OpNoting,
    facts: Mutex<Option<OpFacts>>,
}

impl BoardOperations for Captured<'_> {
    fn execute(
        &self,
        request: ReadRequest<'_>,
        summaries: ListSummaries,
        _: OpNoting,
    ) -> BoardExecution {
        let mut execution = self.inner.execute(request, summaries, self.noting);
        let mut facts = self.facts.lock().unwrap_or_else(PoisonError::into_inner);
        *facts = execution.facts.take();
        execution
    }

    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        self.inner.left_runs()
    }
}
