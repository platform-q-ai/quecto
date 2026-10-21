//! The completion delivery check's contract (#2666), run against its
//! adapter over the project board's remote revisions: only a board run's
//! coordinator is judged, against the repository its session's board tool
//! registered; its completion commit (`completed at <revision>`, read as
//! the push rule reads a revision) is on the remote or not; a completion
//! naming no commit the rule checks, or a board, repository or remote that
//! cannot be read, is unknown (so the box is kept).
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use quecto::application::environments::dto::SwarmBoardSnapshot;
use quecto::application::environments::ports::hosted_swarm::CompletionDeliveryCheck;
use quecto::application::project_board::ports::{BoardStoreError, RemoteRevisions};
use quecto::domain::environments::services::environment_retention::CompletionDelivery;
use quecto::infrastructure::tools::agents::coordinator_launch::{
    RemoteCompletionDelivery, register_board_repository,
};
use serde_json::json;

const SHA: &str = "2b23fe18d39ae8692eaab45d60cc50ff1d66c1d3";

/// The remote every opened repository reads: it holds every commit but
/// `SHA` when `missing`, or is down; it notes its asks. One per process,
/// as the adapter takes a plain `fn` to open a repository.
struct Remote {
    asked: Mutex<Vec<(String, Vec<String>)>>,
}

fn remote() -> &'static Arc<Remote> {
    static REMOTE: OnceLock<Arc<Remote>> = OnceLock::new();
    REMOTE.get_or_init(|| {
        Arc::new(Remote {
            asked: Mutex::new(Vec::new()),
        })
    })
}

/// The repository a remote is opened for: its name says how it answers
/// (`missing`, `down`, `fine`).
struct Opened(String);

impl RemoteRevisions for Opened {
    fn not_on_remote(&self, revisions: &[String]) -> Result<Vec<String>, BoardStoreError> {
        remote()
            .asked
            .lock()
            .unwrap()
            .push((self.0.clone(), revisions.to_vec()));
        match self.0.split('-').next() {
            Some("down") => Err(BoardStoreError::Unavailable("the remote is down".into())),
            Some("missing") => Ok(revisions.to_vec()),
            _ => Ok(vec![]),
        }
    }

    fn commit_id_hex_len(&self) -> Result<usize, BoardStoreError> {
        Ok(40)
    }
}

fn open(repo: &Path) -> Result<Arc<dyn RemoteRevisions>, String> {
    let name = repo.file_name().unwrap().to_str().unwrap().to_owned();
    match name.split('-').next() {
        Some("absent") => Err("no origin here".into()),
        Some("flaky") => {
            let mut opens = flaky_opens().lock().unwrap();
            *opens += 1;
            match *opens {
                1 => Err("not reachable yet".into()),
                _ => Ok(Arc::new(Opened(name))),
            }
        }
        _ => Ok(Arc::new(Opened(name))),
    }
}

/// How many times a `flaky` repository was opened: its first open fails.
fn flaky_opens() -> &'static Mutex<usize> {
    static OPENS: OnceLock<Mutex<usize>> = OnceLock::new();
    OPENS.get_or_init(Mutex::default)
}

/// The revisions `repo`'s remote was asked about, in order.
fn asks_of(repo: &str) -> Vec<Vec<String>> {
    remote()
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|(asked, _)| asked == repo)
        .map(|(_, revisions)| revisions.clone())
        .collect()
}

fn board(reason: Option<&str>) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({"run": {"status": "succeeded", "outcome_reason": reason}}))
        .unwrap()
}

/// What the check says of `session`'s board run 1 of `a`, whose
/// repository is `repo`, its board's completion `reason`.
fn judged(session: &str, repo: &str, reason: Option<&str>) -> CompletionDelivery {
    register_board_repository(session, Path::new(&format!("/repos/{repo}")));
    let check = RemoteCompletionDelivery::new(open);
    futures::executor::block_on(check.delivery(session, "board-a-run-1", Some(&board(reason))))
}

fn at(revision: &str) -> String {
    format!("completed at {revision}")
}

#[test]
fn a_completion_commit_is_on_the_remote_or_not() {
    assert_eq!(
        judged("cd-1", "fine-1", Some(&at(SHA))),
        CompletionDelivery::OnRemote
    );
    assert_eq!(asks_of("fine-1"), [vec![SHA.to_owned()]], "asked once");
    assert_eq!(
        judged(
            "cd-2",
            "missing",
            Some(&at(&format!("`{}`.", SHA.to_uppercase())))
        ),
        CompletionDelivery::NotOnRemote {
            revision: SHA.to_owned()
        }
    );
}

/// Round 3 probes: words around the commit (short hex-only ones such as
/// `a`, `2`, `added`, `be`, `cafe`) and trailing punctuation still read
/// the one commit.
#[test]
fn a_completion_commit_is_read_out_of_the_words_around_it() {
    for reason in [
        format!("{SHA} added fib"),
        format!("{SHA} for 2 items"),
        format!("{SHA} (a fix)"),
        format!("`{SHA}`."),
        format!("commit {SHA}, be cafe"),
        format!("{SHA} on 20261021"),
        format!("owner/repo@{SHA}"),
    ] {
        assert_eq!(
            judged("cd-3", "missing", Some(&at(&reason))),
            CompletionDelivery::NotOnRemote {
                revision: SHA.to_owned()
            },
            "{reason}"
        );
    }
}

#[test]
fn only_a_board_runs_completion_is_judged() {
    register_board_repository("cd-4", Path::new("/repos/missing-4"));
    let check = RemoteCompletionDelivery::new(open);
    assert_eq!(
        futures::executor::block_on(check.delivery(
            "cd-4",
            "my-ad-hoc-swarm",
            Some(&board(Some(&at(SHA))))
        )),
        CompletionDelivery::NotJudged
    );
    assert!(asks_of("missing-4").is_empty(), "never asked");
}

/// What cannot be read, and a completion naming no commit the push rule
/// checks (none, two, a short id, a SHA-256 digest in a SHA-1
/// repository), is unknown: the box is kept.
#[test]
fn what_cannot_be_read_or_checked_is_unknown() {
    let digest = "e".repeat(64);
    let cases: Vec<(&str, &str, Option<String>)> = vec![
        ("cd-5", "down", Some(at(SHA))),
        ("cd-6", "absent", Some(at(SHA))),
        ("cd-7", "fine", None),
        ("cd-8", "fine", Some("closed by owner".into())),
        ("cd-9", "fine", Some(at("red-tests.bundle"))),
        ("cd-10", "fine", Some(at("abc1234"))),
        ("cd-11", "fine", Some(at(&format!("{SHA} {SHA}")))),
        ("cd-12", "fine", Some(at(&digest))),
    ];
    for (session, repo, reason) in cases {
        assert!(
            matches!(
                judged(session, repo, reason.as_deref()),
                CompletionDelivery::Unknown { .. }
            ),
            "{repo} {reason:?}"
        );
    }
    let check = RemoteCompletionDelivery::new(open);
    assert!(matches!(
        futures::executor::block_on(check.delivery(
            "cd-unregistered",
            "board-a-run-1",
            Some(&board(Some(&at(SHA))))
        )),
        CompletionDelivery::Unknown { .. }
    ));
    assert!(matches!(
        futures::executor::block_on(check.delivery("cd-5", "board-a-run-1", None)),
        CompletionDelivery::Unknown { .. }
    ));
}

#[test]
fn an_unknown_completion_says_why_in_words() {
    match judged("cd-13", "down", Some(&at(SHA))) {
        CompletionDelivery::Unknown { why } => assert_eq!(why, "the remote is down"),
        other => panic!("{other:?}"),
    }
}

/// Round 4: a repository whose remote could not be opened is tried again
/// on the next judgement, not remembered as failed.
#[test]
fn a_remote_that_could_not_be_opened_is_tried_again() {
    register_board_repository("cd-14", Path::new("/repos/flaky-14"));
    let check = RemoteCompletionDelivery::new(open);
    let judge = || {
        futures::executor::block_on(check.delivery(
            "cd-14",
            "board-a-run-1",
            Some(&board(Some(&at(SHA)))),
        ))
    };
    assert!(
        matches!(judge(), CompletionDelivery::Unknown { .. }),
        "first open fails"
    );
    assert_eq!(judge(), CompletionDelivery::OnRemote, "opened on the next");
    assert_eq!(*flaky_opens().lock().unwrap(), 2);
    assert_eq!(judge(), CompletionDelivery::OnRemote);
    assert_eq!(*flaky_opens().lock().unwrap(), 2, "kept once opened");
}
