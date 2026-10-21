//! The completion delivery check's contract (#2666 round 2), run against
//! its adapter over the project board's remote revisions: only a board
//! run's coordinator is judged; its completion commit (`completed at
//! <revision>`, one whole hex word of 12 digits up to a full id) is on the
//! remote or not; a completion naming no commit is not judged; a board,
//! completion or remote that cannot be read is unknown.
use std::sync::{Arc, Mutex};

use quecto::application::environments::dto::SwarmBoardSnapshot;
use quecto::application::environments::ports::hosted_swarm::CompletionDeliveryCheck;
use quecto::application::project_board::ports::{BoardStoreError, RemoteRevisions};
use quecto::domain::environments::services::environment_retention::CompletionDelivery;
use quecto::infrastructure::tools::agents::coordinator_launch::RemoteCompletionDelivery;
use serde_json::json;

const SHA: &str = "2b23fe18d39ae8692eaab45d60cc50ff1d66c1d3";

/// A remote holding every commit but `missing`, or down; it notes asks.
struct Remote {
    missing: Vec<String>,
    down: bool,
    asked: Mutex<Vec<Vec<String>>>,
}

impl RemoteRevisions for Remote {
    fn not_on_remote(&self, revisions: &[String]) -> Result<Vec<String>, BoardStoreError> {
        self.asked.lock().unwrap().push(revisions.to_vec());
        match self.down {
            true => Err(BoardStoreError::Unavailable("down".into())),
            false => Ok(revisions
                .iter()
                .filter(|revision| self.missing.contains(revision))
                .cloned()
                .collect()),
        }
    }

    fn commit_id_hex_len(&self) -> Result<usize, BoardStoreError> {
        Ok(40)
    }
}

fn remote(missing: &[&str], down: bool) -> Arc<Remote> {
    Arc::new(Remote {
        missing: missing.iter().map(|m| (*m).to_owned()).collect(),
        down,
        asked: Mutex::new(Vec::new()),
    })
}

fn board(reason: Option<&str>) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({"run": {"status": "succeeded", "outcome_reason": reason}}))
        .unwrap()
}

fn judged(
    check: &RemoteCompletionDelivery,
    label: &str,
    board: Option<&SwarmBoardSnapshot>,
) -> CompletionDelivery {
    futures::executor::block_on(check.delivery(label, board))
}

fn over(remote: &Arc<Remote>) -> RemoteCompletionDelivery {
    RemoteCompletionDelivery::new(Ok(Arc::clone(remote) as Arc<dyn RemoteRevisions>))
}

#[test]
fn a_completion_commit_is_on_the_remote_or_not() {
    let pushed = remote(&[], false);
    let at = board(Some(&format!("completed at {SHA}")));
    assert_eq!(
        judged(&over(&pushed), "board-a-run-1", Some(&at)),
        CompletionDelivery::OnRemote
    );
    assert_eq!(
        pushed.asked.lock().unwrap().as_slice(),
        &[vec![SHA.to_owned()]]
    );
    let unpushed = remote(&[SHA], false);
    let upper = board(Some(&format!("completed at `{}`", SHA.to_uppercase())));
    assert_eq!(
        judged(&over(&unpushed), "board-a-run-1", Some(&upper)),
        CompletionDelivery::NotOnRemote {
            revision: SHA.to_owned()
        }
    );
}

#[test]
fn only_a_board_runs_commit_completion_is_judged() {
    let asked = remote(&[SHA], false);
    let at = board(Some(&format!("completed at {SHA}")));
    assert_eq!(
        judged(&over(&asked), "my-ad-hoc-swarm", Some(&at)),
        CompletionDelivery::NotJudged
    );
    for reason in [
        "completed at red-tests.bundle",
        "completed at abc1234",
        "completed at main",
    ] {
        assert_eq!(
            judged(&over(&asked), "board-a-run-1", Some(&board(Some(reason)))),
            CompletionDelivery::NotJudged,
            "{reason}"
        );
    }
    assert!(asked.asked.lock().unwrap().is_empty(), "never asked");
}

#[test]
fn what_cannot_be_read_is_unknown() {
    let down = remote(&[], true);
    let at = board(Some(&format!("completed at {SHA}")));
    for (check, board) in [
        (over(&down), Some(at.clone())),
        (over(&remote(&[], false)), None),
        (over(&remote(&[], false)), Some(board(None))),
        (
            over(&remote(&[], false)),
            Some(board(Some("closed by owner"))),
        ),
        (
            RemoteCompletionDelivery::new(Err("no origin here".into())),
            Some(at.clone()),
        ),
    ] {
        assert!(
            matches!(
                judged(&check, "board-a-run-1", board.as_ref()),
                CompletionDelivery::Unknown { .. }
            ),
            "{board:?}"
        );
    }
}
