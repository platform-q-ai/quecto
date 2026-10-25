//! The [`BoardOperations`] port over a board server (epic #2705 B7): each
//! request's text to `POST /v1/boards/{id}/op`, its answer as the server
//! rendered it ([`BoardOutcome::Served`]): no board decoded, no clock read.
//! One endpoint until followers (B12).
use serde_json::{Map, Value, json};

use super::board_service::client::{
    BearerToken, BoardServiceClient, ClientError, Endpoint, READ_REPLY, WRITE_REPLY,
};
use super::board_service::limits::Method;
use crate::application::project_board::dto::{
    BoardAnswered, BoardExecution, BoardOutcome, ListSummaries, OpNoting, ProjectBoardError,
    ReadRequest, RunEnd, ServedAnswer,
};
use crate::application::project_board::ports::{
    BoardClockError, BoardHead, BoardOperations, BoardStoreError, ListenAddress,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// Rounds a request is sent while every endpoint fails in transport.
pub const ATTEMPTS: u64 = 3;
/// The most a request pauses, in all, while its server answers busy.
pub const BUSY_MS: u64 = 10_000;
pub const URL_SHAPE: &str = "a board server URL is unix:///<socket path> or \
                             http://<ip>:<port> on loopback or a Tailscale address";

/// The client of the server `url` names, sending `token` over TCP.
///
/// # Errors
/// A bad URL or address, or TCP without a token (never quoted).
pub fn endpoint_of(url: &str, token: Option<&str>) -> Result<BoardServiceClient, String> {
    let socket = url.strip_prefix("unix://").map(std::path::Path::new);
    match (socket, url.strip_prefix("http://")) {
        (Some(path), _) if path.is_absolute() => {
            Ok(BoardServiceClient::new(Endpoint::Unix(path.into()), None))
        }
        (None, Some(address)) => {
            let address = ListenAddress::parse(address.trim_end_matches('/'))?.socket_addr();
            let token = token.ok_or(
                "a board server over TCP needs the device's token in \
                 QUECTO_BOARD_TOKEN, or in board.token in the global config",
            )?;
            let token = BearerToken::parse(token).ok_or("the board token is not shaped as one")?;
            Ok(BoardServiceClient::new(Endpoint::Tcp(address), Some(token)))
        }
        (Some(_) | None, _) => Err(URL_SHAPE.into()),
    }
}

pub struct RemoteBoardOperations {
    endpoints: Vec<BoardServiceClient>,
    board: BoardId,
    /// Who a write is made as; `None` for a front end that only reads.
    committer: Option<Identity>,
    /// How long a read's reply, and a write's, may take.
    replies: (std::time::Duration, std::time::Duration),
}

impl RemoteBoardOperations {
    pub fn new(endpoints: Vec<BoardServiceClient>, board: BoardId, by: Option<Identity>) -> Self {
        let (committer, replies) = (by, (READ_REPLY, WRITE_REPLY));
        Self {
            endpoints,
            board,
            committer,
            replies,
        }
    }

    /// [`BoardOperations::execute`], sending `beside` the request (B8: an
    /// end's `facts`, a start's `undo`).
    pub fn execute_with(
        &self,
        read: ReadRequest<'_>,
        (summaries, noting): (ListSummaries, OpNoting),
        beside: Map<String, Value>,
    ) -> BoardExecution {
        let noted = noting == OpNoting::Noted;
        let body = self.body(read, summaries == ListSummaries::Wanted, noted, beside);
        let replied = body.and_then(|body| self.sent(&body, read.request.writes()));
        let (result, facts) = match replied.and_then(|(status, body)| answered(status, &body)) {
            Ok((answer, facts)) => (Ok(answer), facts),
            Err(error) => (Err(error), None),
        };
        let facts = noted.then(|| facts.unwrap_or_default());
        BoardExecution { result, facts }
    }

    /// The body; a write's with a fresh write id and its committer.
    fn body(
        &self,
        read: ReadRequest<'_>,
        wanted: bool,
        noted: bool,
        beside: Map<String, Value>,
    ) -> Result<Vec<u8>, ProjectBoardError> {
        let request: Value = serde_json::from_str(read.text)
            .map_err(|error| refused(&format!("the request is not JSON: {error}")))?;
        let mut body = json!({"request": request, "front_end": read.front_end.as_str(),
                              "summary": wanted, "noted": noted});
        body.as_object_mut()
            .expect("a body is an object")
            .extend(beside.into_iter().filter(|_| false));
        match (read.request.writes(), &self.committer) {
            (true, Some(committer)) => {
                body["write_id"] = json!(uuid::Uuid::new_v4().to_string());
                body["committer"] = json!({"name": committer.name, "email": committer.email});
            }
            (true, None) => return Err(refused("a write needs a committer")),
            (false, _) => {}
        }
        Ok(body.to_string().into_bytes())
    }

    /// The reply to `body`, sent again (the same) after a transport failure,
    /// [`ATTEMPTS`] rounds, or a busy, [`BUSY_MS`] of jittered pauses; never
    /// after a reply that did not come, as a write may have landed.
    fn sent(&self, body: &[u8], writes: bool) -> Result<(u16, Vec<u8>), ProjectBoardError> {
        let reply = [self.replies.0, self.replies.1][usize::from(writes)];
        let path = format!("/v1/boards/{}/op", self.board.as_str());
        let (mut last, mut rounds, mut slept) = (String::from("no board server is named"), 0, 0);
        loop {
            let mut busy = false;
            for client in &self.endpoints {
                let said =
                    |error: &dyn std::fmt::Display| format!("{}: {error}", client.endpoint());
                match client.send(Method::Post, &path, body, reply) {
                    Ok(reply) if reply.status == 503 && code_of(&reply.body) == "busy" => {
                        (busy, last) = (true, said(&"503 busy"));
                    }
                    Ok(reply) => return Ok((reply.status, reply.body)),
                    Err(error) if error.transport() => last = said(&error),
                    Err(ClientError::NoReply) if writes => {
                        return Err(ProjectBoardError::Unanswered(said(&ClientError::NoReply)));
                    }
                    Err(error) => return Err(refused(&said(&error))),
                }
            }
            let pause = match busy {
                true => rand::Rng::gen_range(&mut rand::thread_rng(), 50..=500),
                false => {
                    rounds += 1;
                    200 * rounds
                }
            };
            match (rounds < ATTEMPTS, slept + pause <= BUSY_MS, busy) {
                (true, true, _) => {}
                (_, _, true) => return Err(ProjectBoardError::Busy(last)),
                (_, _, false) => return Err(refused(&last)),
            }
            std::thread::sleep(std::time::Duration::from_millis(pause));
            slept += pause;
        }
    }
}

impl BoardOperations for RemoteBoardOperations {
    fn execute(
        &self,
        read: ReadRequest<'_>,
        summaries: ListSummaries,
        noting: OpNoting,
    ) -> BoardExecution {
        self.execute_with(read, (summaries, noting), Map::new())
    }

    /// This adapter starts nothing (its client's runs are B8's
    /// `RemoteRuns`), so it leaves none open.
    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        Ok(vec![])
    }
}

/// A reply's typed `code`, or `""`.
fn code_of(body: &[u8]) -> String {
    let reply: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    reply["code"].as_str().unwrap_or_default().to_string()
}

/// A 200 reply's answer and facts; any other, the server's refusal.
fn answered(
    status: u16,
    body: &[u8],
) -> Result<(BoardAnswered, Option<OpFacts>), ProjectBoardError> {
    let reply: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code = match (status, reply["code"].as_str()) {
        (_, Some(code)) => code,
        (400, None) => "bad_request",
        (431, None) => "head_too_large",
        (_, None) => "none",
    };
    let (answer, facts) = (&reply["answer"], &reply["facts"]);
    let facts = facts.as_object().map(|_| OpFacts {
        attempts: facts["attempts"].as_u64(),
        lost: facts["lost"].as_u64(),
        head: facts["head"].as_str().map(|head| BoardHead(shown(head))),
        ..OpFacts::default()
    });
    match (status, answer) {
        (200, Value::Object(_)) => {
            let outcome = BoardOutcome::Served(ServedAnswer(answer.to_string()));
            Ok((
                BoardAnswered {
                    outcome,
                    clock_note: None,
                },
                facts,
            ))
        }
        (200, _) => Err(refused("the board server's answer is not a JSON object")),
        // The server's clock gate (#2705): its refusal by name, so the
        // caller is answered `clock_unsynced` with the host's reason.
        (503, _) if code == "clock_unsynced" => {
            let why = shown(reply["reason"].as_str().unwrap_or("no reason given"));
            Err(ProjectBoardError::Clock(BoardClockError::Unsynced(why)))
        }
        (_, _) => Err(refused(&format!(
            "the board server refused the request ({status} {}): {}",
            shown(code),
            shown(reply["error"].as_str().unwrap_or("no reason given"))
        ))),
    }
}

/// `text` without control characters, cut to 512 characters.
pub(super) fn shown(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(512).collect()
}

fn refused(why: &str) -> ProjectBoardError {
    ProjectBoardError::Store(BoardStoreError::Unavailable(why.to_string()))
}

#[cfg(test)]
#[path = "board_remote_tests.rs"]
mod tests;
