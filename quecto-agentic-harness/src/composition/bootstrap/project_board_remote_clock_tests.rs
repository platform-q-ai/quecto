//! The board server's clock gate (epic #2705) over a real SQLite board:
//! while the host's clock is not synchronised, a request that reads the
//! clock is refused `clock_unsynced`, rolled back and kept by no answer.
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::application::project_board::ports::{ClockSync, TimeSync};
use crate::infrastructure::http::board_service::client::BoardServiceClient;
use crate::infrastructure::http::board_service::limits::Method;

const UNSYNCED: &str = "the test host's clock drifts";

/// The host's word on its clock, switched by the test.
pub(super) struct Said(AtomicBool);

impl Said {
    pub(super) fn synced() -> Self {
        Self(AtomicBool::new(true))
    }
}

impl TimeSync for Said {
    fn sync(&self) -> ClockSync {
        match self.0.load(Ordering::SeqCst) {
            true => ClockSync::Synced,
            false => ClockSync::Unsynced(UNSYNCED.into()),
        }
    }
}

fn write(id: &str) -> Value {
    json!({"request": ready(id), "front_end": "cli", "write_id": ID,
           "committer": {"name": "Ada", "email": "ada@example.com"}})
}

#[test]
fn an_unsynchronised_clock_refuses_a_clock_reading_write_and_keeps_no_answer() {
    let said = Arc::new(Said(AtomicBool::new(false)));
    let served = served_with(false, Arc::clone(&said) as Arc<dyn TimeSync>);
    let before = served.revisions().len();
    let raw = BoardServiceClient::unix(&served.socket);
    let (status, refused) = posted(&raw, &served.board, &write("a"));
    assert_eq!(
        (status, &refused["code"], &refused["reason"]),
        (503, &json!("clock_unsynced"), &json!(UNSYNCED)),
        "{refused}"
    );
    assert_eq!(served.revisions().len(), before, "rolled back");
    // The server's time is no authority while it is not synchronised.
    let time = raw.send_read(Method::Get, "/v1/time", b"").unwrap();
    let status = time.status;
    let time: Value = serde_json::from_slice(&time.body).unwrap();
    assert_eq!(
        (status, &time["code"], &time["reason"]),
        (503, &json!("clock_unsynced"), &json!(UNSYNCED))
    );
    // Through the remote adapter, the caller's answer names the refusal.
    let answer = run(&*served.writer(), &ready("b").to_string());
    assert_eq!(
        (&answer["error"]["code"], &answer["error"]["reason"]),
        (&json!("clock_unsynced"), &json!(UNSYNCED)),
        "{answer}"
    );
    // A read that reads the clock is refused alike; one that reads none is
    // answered.
    let takeable = run(
        &*served.writer(),
        &json!({"op": "list_takeable"}).to_string(),
    );
    assert_eq!(
        (&takeable["error"]["code"], &takeable["error"]["reason"]),
        (&json!("clock_unsynced"), &json!(UNSYNCED)),
        "{takeable}"
    );
    let listed = run(&*served.writer(), &json!({"op": "list"}).to_string());
    assert_eq!(listed["tasks"], json!([]), "{listed}");
    // Synchronised again: the same write id runs again (no answer was kept)
    // and lands. The clock is still no outside reference (B7, until B9), so
    // the answer keeps its note.
    said.0.store(true, Ordering::SeqCst);
    let (status, landed) = posted(&raw, &served.board, &write("a"));
    assert_eq!(
        (status, &landed["answer"]["ok"]),
        (200, &json!(true)),
        "{landed}"
    );
    assert_eq!(
        landed["answer"]["clock"]["referenced"],
        json!(false),
        "{landed}"
    );
    assert_eq!(served.revisions().len(), before + 1);
}

/// A synchronised host is no outside reference: the burst bound
/// (`task_ahead`) stays off on the server, as in B7, until B9. So many
/// changes to one task within a second or two all land, though each
/// stamps the task a second past the last.
#[test]
fn a_synchronised_server_takes_a_burst_of_changes_to_one_task() {
    let served = served(false);
    let writer = served.writer();
    let draft = json!({"op": "create", "task": "a", "title": "A", "status": "draft"});
    let created = run(&*writer, &draft.to_string());
    assert_eq!(created["ok"], json!(true), "{created}");
    for at in 0..120 {
        let change = json!({"op": "update_draft", "task": "a", "title": format!("A {at}")});
        let changed = run(&*writer, &change.to_string());
        assert_eq!(changed["ok"], json!(true), "change {at}: {changed}");
    }
}
