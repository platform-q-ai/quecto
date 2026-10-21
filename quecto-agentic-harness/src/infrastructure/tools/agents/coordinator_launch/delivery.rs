//! Where a succeeded board run's work went, read as its owner's kill
//! tears its box down (#2666 round 2): the commit the run completed at
//! (its `outcome_reason`, `completed at <revision>`) asked of the board's
//! remote, through the project board's own [`RemoteRevisions`] port. Only
//! a board run's coordinator ([`super::is_board_label`]) is judged; a
//! completion naming no commit (no one whole hex word of 12 digits up to a
//! full id) is not; one whose board, completion or remote cannot be read
//! is `Unknown`, which keeps the box (fail safe). Blocking: the remote is
//! read inline, as the teardown's scripts run.
use std::sync::Arc;

use crate::application::environments::dto::SwarmBoardSnapshot;
use crate::application::environments::ports::PortFuture;
use crate::application::environments::ports::hosted_swarm::CompletionDeliveryCheck;
use crate::application::project_board::ports::RemoteRevisions;
use crate::domain::environments::services::environment_retention::CompletionDelivery;

/// The shortest commit prefix judged, as the push rule's.
const MIN_PREFIX: usize = 12;

/// Judges completions against the board's remote, or says why it cannot.
pub struct RemoteCompletionDelivery {
    remote: Result<Arc<dyn RemoteRevisions>, String>,
}

impl RemoteCompletionDelivery {
    /// Over `remote`, or the reason there is none (no board's repository
    /// or remote here): every board run's completion is then unknown.
    pub fn new(remote: Result<Arc<dyn RemoteRevisions>, String>) -> Self {
        Self { remote }
    }

    fn judged(&self, label: &str, board: Option<&SwarmBoardSnapshot>) -> CompletionDelivery {
        match super::is_board_label(label) {
            true => {}
            false => return CompletionDelivery::NotJudged,
        }
        let unknown = |why: &str| CompletionDelivery::Unknown {
            why: why.to_owned(),
        };
        let Some(board) = board else {
            return unknown("its board could not be read");
        };
        let board = board.clone().into_value();
        let reason = board
            .get("run")
            .and_then(|run| run.get("outcome_reason"))
            .and_then(serde_json::Value::as_str);
        let Some(revision) = reason.and_then(|reason| reason.strip_prefix("completed at ")) else {
            return unknown("it recorded no completion revision");
        };
        let Some(token) = commit_token(revision) else {
            return CompletionDelivery::NotJudged;
        };
        let remote = match &self.remote {
            Ok(remote) => remote,
            Err(why) => return unknown(why),
        };
        let id_len = match remote.commit_id_hex_len() {
            Ok(id_len) => id_len,
            Err(error) => return unknown(&format!("{error:?}")),
        };
        match (MIN_PREFIX..=id_len).contains(&token.len()) {
            true => {}
            false => return CompletionDelivery::NotJudged,
        }
        match remote.not_on_remote(std::slice::from_ref(&token)) {
            Ok(missing) => match missing.as_slice() {
                [] => CompletionDelivery::OnRemote,
                [_, ..] => CompletionDelivery::NotOnRemote { revision: token },
            },
            Err(error) => unknown(&format!("{error:?}")),
        }
    }
}

/// The one whitespace-separated word of `revision`, backticks trimmed,
/// that is wholly hex digits, in lower case; none when no word is, or two
/// are.
fn commit_token(revision: &str) -> Option<String> {
    let mut hex = revision
        .split_whitespace()
        .map(|word| word.trim_matches('`'))
        .filter(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_hexdigit()));
    match (hex.next(), hex.next()) {
        (Some(token), None) => Some(token.to_ascii_lowercase()),
        (Some(_), Some(_)) | (None, _) => None,
    }
}

impl CompletionDeliveryCheck for RemoteCompletionDelivery {
    fn delivery<'a>(
        &'a self,
        label: &'a str,
        board: Option<&'a SwarmBoardSnapshot>,
    ) -> PortFuture<'a, CompletionDelivery> {
        let judged = self.judged(label, board);
        Box::pin(async move { judged })
    }
}
