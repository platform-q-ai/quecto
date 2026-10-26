//! `POST /v1/boards/{id}/op` (epic #2705 B7): a request as its front end
//! read it, answered through the front ends' own `run_recorded_with`.
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use crate::application::project_board::dto::{
    BoardExecution, BoardRequest, ListSummaries, OpNoting, ProjectBoardError, ReadRequest, RunEnd,
};
use crate::application::project_board::ports::{
    BoardClockError, BoardOperations, OpWrite, ServeError, ServedRun, Transacted, WriteId,
};
use crate::application::project_board::run_facts::RunFacts;
use crate::application::project_board::start::StartUndo;
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::{
    BoardAccess, Principal, Refused, Scope, ServeBoardOperation,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::interface::tools::project_board::recording::BoardCall;
use crate::interface::tools::project_board::request::{entry_policy, parse};
use crate::interface::tools::project_board::{read, run_recorded_with};

const BODY_SHAPE: &str = "the body must be {\"request\": {..}, \"front_end\": \
     \"cli\" or \"tool\", \"summary\", \"noted\", and for a write \"write_id\" and \
     \"committer\": {\"name\", \"email\"}; an end_run may add its \"facts\", a start \
     its \"undo\": {\"run\", \"from\", \"reason\"}}";

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
    /// What the client saw of an end's run (B8).
    facts: Option<Value>,
    /// A start's undo, its launch having failed (B8).
    undo: Option<Undo>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Committer {
    name: String,
    email: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Undo {
    run: u32,
    from: String,
    reason: String,
}

/// What `facts` and `undo` say for `request`'s run: facts only with an
/// end (an end without them saw nothing and kept nothing), an undo only
/// with a start; `None` for any other pairing.
fn served_run(
    request: Option<&BoardRequest>,
    facts: Option<&Value>,
    undo: Option<&Undo>,
) -> Option<ServedRun> {
    match (request, facts, undo) {
        (Some(BoardRequest::EndRun(_)), facts, None) => {
            let facts = facts.map_or(Ok(RunFacts::unseen()), |facts| {
                serde_json::from_value(facts.clone())
            });
            facts.ok().map(ServedRun::Facts)
        }
        (Some(BoardRequest::Start(start)), None, Some(undo)) => {
            let from = TaskStatus::parse(&undo.from)?;
            Some(ServedRun::Undo(StartUndo {
                task: start.task.clone(),
                plan_version: start.plan_version,
                run: undo.run,
                from,
                reason: crate::interface::tools::project_board::answer::shown(&undo.reason),
            }))
        }
        (_, None, None) => Some(ServedRun::Plain),
        (_, Some(_), _) | (_, None, Some(_)) => None,
    }
}

/// Why an op was not answered: a body not an op's (why), access, a write
/// to a follower (B11), or the server.
pub(super) enum OpRefusal {
    Body(String),
    Access(Refused),
    Standby,
    Serve(ServeError),
}

/// The answer to the op `body` from `principal`; on a `standby` (a
/// follower, B11) only a read is answered.
///
/// # Errors
/// A body not an op's, an op `principal` may not make, a write to a
/// standby, or the server's.
pub(super) fn answer_op(
    operations: &ServeBoardOperation,
    principal: &Principal,
    body: &[u8],
    standby: bool,
) -> Result<String, OpRefusal> {
    let shape = || OpRefusal::Body(BODY_SHAPE.into());
    let text = std::str::from_utf8(body).map_err(|_| shape())?;
    let body: OpBody =
        serde_json::from_value(read(text).map_err(|_| shape())?).map_err(|_| shape())?;
    let front_end = FrontEnd::ALL
        .iter()
        .copied()
        .find(|named| named.as_str() == body.front_end);
    let (Some(front_end), Value::Object(_)) = (front_end, &body.request) else {
        return Err(shape());
    };
    let request = parse(&body.request, entry_policy(front_end)).ok();
    let scope = request.as_ref().map_or(Scope::Read, Scope::of);
    BoardAccess::authorize(principal, scope).map_err(OpRefusal::Access)?;
    match (standby, scope) {
        (false, _) | (true, Scope::Read | Scope::Replicate) => {}
        (true, Scope::Write | Scope::Admin) => return Err(OpRefusal::Standby),
    }
    let text = body.request.to_string();
    let sent = served_run(request.as_ref(), body.facts.as_ref(), body.undo.as_ref());
    let sent = sent.ok_or_else(shape)?;
    // A write id is bound to its run's facts or undo too.
    let beside = json!({"facts": body.facts, "undo": body.undo}).to_string();
    let writes = request.as_ref().map(BoardRequest::writes);
    let write = match (writes, body.write_id, body.committer) {
        (Some(true), Some(id), Some(Committer { name, email })) => {
            let unshaped = || OpRefusal::Body("write_id must be a lowercase UUID".into());
            let committer = Identity { name, email };
            let invalid = |error| OpRefusal::Body(format!("the committer is invalid: {error}"));
            committer.validate("committer").map_err(invalid)?;
            Some(OpWrite {
                write_id: WriteId::parse(&id).ok_or_else(unshaped)?,
                request: format!("{}\n{text}\n{beside}", front_end.as_str()),
                committer,
                principal: principal.recorded(),
            })
        }
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
    // The clock gate's reason, when its refusal undid the op (#2705).
    let mut unsynced = None;
    let mut answer = |board: &dyn BoardOperations| {
        present(board, &text, front_end, summaries, noting).map_err(|unkept| {
            unsynced = unkept.unsynced;
            unkept.message
        })
    };
    let executed = operations.execute_run(request.as_ref(), write.as_ref(), &sent, &mut answer);
    match (executed, unsynced) {
        (Ok(Transacted::Answered(answer) | Transacted::Replayed(answer)), _) => Ok(answer),
        (Ok(Transacted::Reused), _) => Err(OpRefusal::Serve(ServeError::WriteIdReused)),
        (Err(ServeError::Board(_)), Some(why)) => {
            let unsynced = ProjectBoardError::Clock(BoardClockError::Unsynced(why));
            Err(OpRefusal::Serve(ServeError::Board(unsynced)))
        }
        (Err(error), _) => Err(OpRefusal::Serve(error)),
    }
}

/// Why an op keeps no answer: a store-class refusal's message, and the
/// host's reason when it is the clock gate's (`clock_unsynced`).
struct Unkept {
    message: String,
    unsynced: Option<String>,
}

/// The reply body (answer and facts), or why the store or clock failed
/// under it (the op is then undone).
fn present(
    board: &dyn BoardOperations,
    text: &str,
    front_end: FrontEnd,
    summaries: ListSummaries,
    noting: OpNoting,
) -> Result<String, Unkept> {
    let facts = Mutex::new(None);
    let captured = Captured {
        board,
        noting,
        facts,
    };
    let mut call = BoardCall::start(None, front_end, Client::Other);
    let answer = run_recorded_with(&captured, text, summaries, &mut call);
    let error = &answer["error"];
    // The refusal's own reason: the reply names the store's failure once.
    let message = error["reason"].as_str().or(error["message"].as_str());
    let message = message.unwrap_or_default().to_string();
    match error["code"].as_str() {
        Some("store_unavailable" | "clock_unreadable") => {
            return Err(Unkept {
                message,
                unsynced: None,
            });
        }
        Some("clock_unsynced") => {
            let why = error["reason"].as_str().unwrap_or_default().to_string();
            return Err(Unkept {
                message,
                unsynced: Some(why),
            });
        }
        Some(_) | None => {}
    }
    let facts = captured
        .facts
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let facts = facts.map(|facts| {
        json!({"attempts": facts.attempts, "lost": facts.lost,
               "head": facts.head.map(|head| head.0)})
    });
    Ok(json!({ "answer": answer, "facts": facts }).to_string())
}

/// The operations, noted as the client asked, their facts kept.
struct Captured<'a> {
    board: &'a dyn BoardOperations,
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
        let mut execution = self.board.execute(request, summaries, self.noting);
        *self.facts.lock().unwrap_or_else(PoisonError::into_inner) = execution.facts.take();
        execution
    }

    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        self.board.left_runs()
    }
}

#[cfg(test)]
#[path = "op_tests.rs"]
mod tests;
