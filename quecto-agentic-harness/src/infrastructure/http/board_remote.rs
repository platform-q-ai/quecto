//! The board's operations over a board server (epic #2705 B7): the
//! [`BoardOperations`] port as `POST /v1/boards/{id}/op`. A request goes
//! as the text its front end read, and the answer comes back as the
//! server rendered it ([`BoardOutcome::Served`]): the client decodes no
//! board and reads no clock. A write carries an id made once per request
//! and the committer it asserts; after a transport failure it is sent
//! again with the same id, so the server answers it with its first
//! answer rather than writing twice. The endpoints are tried in order;
//! there is one until followers (B12) add the standby's.
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

/// How many rounds a request is sent while every endpoint fails in transport.
pub const ATTEMPTS: u32 = 3;

/// Why a board server URL is refused.
pub const URL_SHAPE: &str = "a board server URL is unix:///<socket path> or \
                             http://<ip>:<port> on loopback or a Tailscale address";

/// The client of the server `url` names, sending `token` over TCP.
///
/// # Errors
/// A URL of neither shape, an address off the allowlist, or TCP without a
/// token shaped as one (no reason quotes a token).
pub fn endpoint_of(url: &str, token: Option<&str>) -> Result<BoardServiceClient, String> {
    let socket = url.strip_prefix("unix://").map(std::path::Path::new);
    match (socket, url.strip_prefix("http://")) {
        (Some(path), _) if path.is_absolute() => {
            Ok(BoardServiceClient::new(Endpoint::Unix(path.into()), None))
        }
        (None, Some(address)) => {
            let address = ListenAddress::parse(address.trim_end_matches('/'))?;
            let token = token.ok_or(
                "a board server over TCP needs a token: set \
                 QUECTO_BOARD_TOKEN, or board.token in the global config",
            )?;
            let token = BearerToken::parse(token).ok_or("the board token is not shaped as one")?;
            Ok(BoardServiceClient::new(
                Endpoint::Tcp(address.socket_addr()),
                Some(token),
            ))
        }
        (Some(_) | None, _) => Err(URL_SHAPE.into()),
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

    /// The request's body: its text and front end, summaries and noting,
    /// and for a write a fresh write id and its committer.
    fn body(
        &self,
        read: ReadRequest<'_>,
        wanted: bool,
        noted: bool,
    ) -> Result<Vec<u8>, ProjectBoardError> {
        let request: Value = serde_json::from_str(read.text)
            .map_err(|error| refused(&format!("the request is not JSON: {error}")))?;
        let mut body = json!({"request": request, "front_end": read.front_end.as_str(),
                              "summary": wanted, "noted": noted});
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

    /// The reply to `body`: each endpoint in turn while it fails in
    /// transport, at most [`ATTEMPTS`] rounds, the same body each time.
    fn sent(&self, body: &[u8]) -> Result<(u16, Vec<u8>), ProjectBoardError> {
        let path = format!("/v1/boards/{}/op", self.board.as_str());
        let mut last = String::from("no board server is named");
        for attempt in 0..ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)));
            for client in &self.endpoints {
                let said = |error: &ClientError| format!("{}: {error}", client.endpoint());
                match client.send(Method::Post, &path, body) {
                    Ok(reply) => return Ok((reply.status, reply.body)),
                    Err(error) if error.transport() => last = said(&error),
                    Err(error) => return Err(refused(&said(&error))),
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
        let noted = noting == OpNoting::Noted;
        let replied = self
            .body(read, summaries == ListSummaries::Wanted, noted)
            .and_then(|body| self.sent(&body))
            .and_then(|(status, body)| answered(status, &body));
        let (result, facts) = match replied {
            Ok((answer, facts)) => {
                let outcome = BoardOutcome::Served(ServedAnswer(answer));
                let answered = BoardAnswered {
                    outcome,
                    clock_note: None,
                };
                (Ok(answered), facts)
            }
            Err(error) => (Err(error), None),
        };
        BoardExecution {
            result,
            facts: noted.then(|| facts.unwrap_or_default()),
        }
    }

    /// A start is not served until B8, so no run of this front end's is
    /// left open on a served board.
    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        Ok(vec![])
    }
}

/// The answer and facts of a 200 reply `body`; any other `status` is the
/// server's refusal, by its code (a bodiless 400 or 431 is hyper's).
fn answered(status: u16, body: &[u8]) -> Result<(String, Option<OpFacts>), ProjectBoardError> {
    let reply: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code = match (status, reply["code"].as_str()) {
        (_, Some(code)) => code,
        (400, None) => "bad_request",
        (431, None) => "head_too_large",
        (_, None) => "none",
    };
    match (status, &reply["answer"], &reply["facts"]) {
        (200, answer @ Value::Object(_), facts) => Ok((answer.to_string(), facts_of(facts))),
        (200, _, _) => Err(refused("the board server's answer is not a JSON object")),
        (_, _, _) => Err(refused(&format!(
            "the board server refused the request ({status} {}): {}",
            shown(code),
            shown(reply["error"].as_str().unwrap_or("no reason given"))
        ))),
    }
}

/// The facts a server noted, as far as they travel: attempts, races lost
/// and the revision written.
fn facts_of(facts: &Value) -> Option<OpFacts> {
    facts.as_object().map(|_| OpFacts {
        attempts: facts["attempts"].as_u64(),
        lost: facts["lost"].as_u64(),
        head: facts["head"].as_str().map(|head| BoardHead(shown(head))),
        ..OpFacts::default()
    })
}

/// `text` without control characters, cut to 512 characters.
fn shown(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(512).collect()
}

fn refused(why: &str) -> ProjectBoardError {
    ProjectBoardError::Store(BoardStoreError::Unavailable(why.to_string()))
}

#[cfg(test)]
#[path = "board_remote_tests.rs"]
mod tests;
