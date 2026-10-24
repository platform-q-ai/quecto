//! The board's operations over a board server (epic #2705 B7): the
//! [`BoardOperations`] port as `POST /v1/boards/{id}/op`. Each request
//! goes as the text its front end read, with that front end's name, and
//! the server's answer comes back as it rendered it
//! ([`BoardOutcome::Served`]): the client decodes no board and reads no
//! clock. A write carries a write id made once per request and the
//! committer it asserts; after a transport failure it is sent again, with
//! the same id, so the server answers it with its first answer rather
//! than writing it twice.
//!
//! The endpoints are tried in order, each attempt; there is one until
//! followers (B12) add the standby's.
use serde_json::{Value, json};

use super::board_service::client::{BearerToken, BoardServiceClient, ClientError, Endpoint};
use super::board_service::limits::Method;
use crate::application::project_board::dto::{
    BoardAnswered, BoardExecution, BoardOutcome, ListSummaries, OpNoting, ProjectBoardError,
    ReadRequest, RunEnd, ServedAnswer,
};
use crate::application::project_board::ports::{
    BoardHead, BoardOperations, BoardStoreError, ListenAddress,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// How many times a request is sent while each attempt fails in transport.
pub const ATTEMPTS: u32 = 3;

/// Why a board server URL is refused (`--url`, `QUECTO_BOARD_URL`,
/// `board.url`).
pub const URL_SHAPE: &str = "a board server URL is unix:///<socket path> or \
                             http://<ip>:<port> on loopback or a Tailscale address";

/// The endpoint `url` names, with `token` for one over TCP.
///
/// # Errors
/// A URL of neither shape, a TCP address off the allowlist, or TCP
/// without a token (its reason says where a token is read from, never
/// any token).
pub fn endpoint_of(url: &str, token: Option<&str>) -> Result<BoardServiceClient, String> {
    if let Some(path) = url.strip_prefix("unix://") {
        let path = std::path::Path::new(path);
        return match path.is_absolute() {
            true => Ok(BoardServiceClient::new(Endpoint::Unix(path.into()), None)),
            false => Err(URL_SHAPE.into()),
        };
    }
    let Some(address) = url.strip_prefix("http://") else {
        return Err(URL_SHAPE.into());
    };
    let address = ListenAddress::parse(address.trim_end_matches('/'))?;
    match token.map(BearerToken::parse) {
        Some(Some(token)) => Ok(BoardServiceClient::new(
            Endpoint::Tcp(address.socket_addr()),
            Some(token),
        )),
        Some(None) => Err("the board server's token is not shaped as one".into()),
        None => Err(
            "a board server over TCP needs a token: set QUECTO_BOARD_TOKEN, \
                     or board.token in the global config"
                .into(),
        ),
    }
}

/// The board's operations on the board server at `endpoints`.
pub struct RemoteBoardOperations {
    endpoints: Vec<BoardServiceClient>,
    board: BoardId,
    /// Who a write is made as; `None` for a front end that only reads.
    committer: Option<Identity>,
}

impl RemoteBoardOperations {
    pub fn new(
        endpoints: Vec<BoardServiceClient>,
        board: BoardId,
        committer: Option<Identity>,
    ) -> Self {
        Self {
            endpoints,
            board,
            committer,
        }
    }

    /// The request's body: its text, front end, summaries and noting, and
    /// for a write its id and committer.
    fn body(
        &self,
        read: ReadRequest<'_>,
        summaries: ListSummaries,
        noting: OpNoting,
    ) -> Result<Vec<u8>, ProjectBoardError> {
        let request: Value = serde_json::from_str(read.text)
            .map_err(|error| refused(&format!("the request is not JSON: {error}")))?;
        let mut body = json!({
            "request": request,
            "front_end": read.front_end.as_str(),
            "summary": summaries == ListSummaries::Wanted,
            "noted": noting == OpNoting::Noted,
        });
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

    /// The server's reply to `body`, sent to each endpoint in turn while
    /// it fails in transport, at most [`ATTEMPTS`] rounds.
    fn sent(&self, body: &[u8]) -> Result<(u16, Vec<u8>), ProjectBoardError> {
        let path = format!("/v1/boards/{}/op", self.board.as_str());
        let mut last = String::from("no board server is named");
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)));
            }
            for client in &self.endpoints {
                match client.send(Method::Post, &path, body) {
                    Ok(reply) => return Ok((reply.status, reply.body)),
                    Err(error) if error.transport() => last = said(client, &error),
                    Err(error) => return Err(refused(&said(client, &error))),
                }
            }
        }
        Err(refused(&last))
    }
}

impl BoardOperations for RemoteBoardOperations {
    fn execute(
        &self,
        read: ReadRequest<'_>,
        summaries: ListSummaries,
        noting: OpNoting,
    ) -> BoardExecution {
        let noted = |facts: Option<OpFacts>| match noting {
            OpNoting::Noted => Some(facts.unwrap_or_default()),
            OpNoting::Unnoted => None,
        };
        let replied = self
            .body(read, summaries, noting)
            .and_then(|body| self.sent(&body))
            .and_then(|(status, body)| answered(status, &body));
        match replied {
            Ok((answer, facts)) => BoardExecution {
                result: Ok(BoardAnswered {
                    outcome: BoardOutcome::Served(ServedAnswer(answer)),
                    clock_note: None,
                }),
                facts: noted(facts),
            },
            Err(error) => BoardExecution {
                result: Err(error),
                facts: noted(None),
            },
        }
    }

    /// A start is not served until B8, so no run of this front end's is
    /// left open on a served board.
    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        Ok(vec![])
    }
}

/// The answer and facts of a 200 reply `body`; any other `status` is the
/// server's refusal, by its code.
fn answered(status: u16, body: &[u8]) -> Result<(String, Option<OpFacts>), ProjectBoardError> {
    let reply: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    match (status, &reply["answer"]) {
        (200, answer @ Value::Object(_)) => Ok((answer.to_string(), facts_of(&reply["facts"]))),
        (200, _) => Err(refused("the board server's answer is not a JSON object")),
        (status, _) => {
            let code = match (status, reply["code"].as_str()) {
                (_, Some(code)) => code,
                (400, None) => "bad_request",
                (431, None) => "head_too_large",
                (_, None) => "none",
            };
            let why = reply["error"].as_str().unwrap_or("no reason given");
            Err(refused(&format!(
                "the board server refused the request ({status} {}): {}",
                shown(code),
                shown(why)
            )))
        }
    }
}

/// The facts a server noted, as far as they travel: attempts, races lost
/// and the head written.
fn facts_of(facts: &Value) -> Option<OpFacts> {
    let Value::Object(_) = facts else {
        return None;
    };
    let head = facts["head"].as_str().map(|head| BoardHead(shown(head)));
    Some(OpFacts {
        attempts: facts["attempts"].as_u64(),
        lost: facts["lost"].as_u64(),
        head,
        ..OpFacts::default()
    })
}

/// `text` cut to 512 characters of the visible ones and spaces: a reply's
/// words never carry a control character into an answer.
fn shown(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(512).collect()
}

/// `error` from `client`, saying where (never a token).
fn said(client: &BoardServiceClient, error: &ClientError) -> String {
    format!("{}: {error}", client.endpoint())
}

fn refused(why: &str) -> ProjectBoardError {
    ProjectBoardError::Store(BoardStoreError::Unavailable(why.to_string()))
}

#[cfg(test)]
#[path = "board_remote_tests.rs"]
mod tests;
