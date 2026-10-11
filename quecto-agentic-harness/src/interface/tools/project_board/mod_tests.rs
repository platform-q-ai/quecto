use super::*;
use crate::application::project_board::operations::query_listings::{QueryListings, assembled};
use crate::application::project_board::ports::{
    BoardClock, BoardClockError, PullRequestState, ReferenceStatus,
};
use crate::application::project_board::use_cases::QueryBoard;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

/// A clock reading one time, with or without an outside reference.
struct Fixed {
    at: Timestamp,
    referenced: bool,
}

impl BoardClock for Fixed {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        Ok(self.at.clone())
    }

    fn reference(&self) -> Option<Timestamp> {
        self.referenced.then(|| self.at.clone())
    }

    fn reference_status(&self) -> ReferenceStatus {
        match self.referenced {
            true => ReferenceStatus::Referenced,
            false => ReferenceStatus::NoReference("no outside time in this test".into()),
        }
    }

    fn pause(&self, _: usize) {}

    fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project_board/operations.json")
}

/// The fixture's requests, run in order against one board, answer exactly
/// what it holds: the answer shapes are a contract with the CLI's and the
/// tool's clients. A case may run `as` another committer, `at` another
/// time, with `no_reference` on the clock, after setting a task aside
/// (`quarantine`) or a pull request merged (`merge`).
/// `QUECTO_BLESS_BOARD_FIXTURES=1` (that value only) rewrites the file, and
/// fails the run so a blessed file is always looked at before it passes. A
/// request given as a JSON string is sent as that text.
#[test]
fn every_answer_keeps_its_stable_shape() {
    let store = MemoryBoardStore::default();
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    host.set(8, PullRequestState::Open);
    host.set(9, PullRequestState::Open);
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_path()).unwrap()).unwrap();
    let cases = fixture.as_array().unwrap();
    let mut answered = Vec::new();
    for case in cases {
        if let Some(id) = case.get("quarantine").and_then(Value::as_str) {
            store.quarantine(id, "malformed");
        }
        if let Some(pr) = case.get("merge").and_then(Value::as_u64) {
            host.set(pr, PullRequestState::Merged);
        }
        let name = case.get("as").and_then(Value::as_str).unwrap_or("Ada");
        let at = case.get("at").and_then(Value::as_str).unwrap_or("10:00:00");
        let client = store.client_as(Identity {
            name: name.into(),
            email: format!("{}@example.com", name.to_lowercase()),
        });
        let clock = Fixed {
            at: Timestamp::parse(&format!("2026-10-10T{at}Z")).unwrap(),
            referenced: case.get("no_reference").is_none(),
        };
        let operations = assembled(
            Arc::new(client.clone()),
            Arc::new(clock),
            Arc::new(QueryListings(QueryBoard::new(Arc::new(client)))),
            Ok(Arc::new(host.clone())),
        )
        .unwrap();
        let mut done = case.as_object().unwrap().clone();
        let text = match &case["request"] {
            // A request given as text is sent as it is (a duplicate key,
            // say, which a JSON value cannot hold).
            Value::String(text) => text.clone(),
            request => request.to_string(),
        };
        done.insert("answer".into(), run(&operations, &text));
        answered.push(Value::Object(done));
    }
    if std::env::var("QUECTO_BLESS_BOARD_FIXTURES").as_deref() == Ok("1") {
        let text = serde_json::to_string_pretty(&answered).unwrap() + "\n";
        std::fs::write(fixture_path(), text).unwrap();
        panic!("the board answer fixtures were rewritten: look them over, then run again");
    }
    for (case, got) in cases.iter().zip(&answered) {
        assert_eq!(case["answer"], got["answer"], "{}", case["request"]);
    }
}

#[test]
fn a_command_line_reaches_the_same_request_as_the_tools_json() {
    let line = |op: &str, flags: &[&str]| {
        let flags: Vec<String> = flags.iter().map(|flag| (*flag).to_string()).collect();
        command_line::request(op, &flags)
    };
    assert_eq!(
        line("link-pr", &["--task", "a", "--pr", "2493"]),
        Ok(json!({"op": "link_pr", "task": "a", "pr": 2493}))
    );
    assert_eq!(
        line(
            "create",
            &["--task", "a", "--title", "123", "--depends-on", "b,c"]
        ),
        Ok(json!({"op": "create", "task": "a", "title": "123", "depends_on": ["b", "c"]}))
    );
    assert_eq!(
        line(
            "update-draft",
            &["--task", "a", "--parent", "", "--depends-on", ""]
        ),
        Ok(json!({"op": "update_draft", "task": "a", "parent": null, "depends_on": []}))
    );
    assert_eq!(
        line("move", &["--task", "a", "--to", "ready"]),
        Ok(json!({"op": "move", "task": "a", "to": "ready"}))
    );
    let pr_word = line("link-pr", &["--task", "a", "--pr", "twelve"]).unwrap();
    assert_eq!(
        request::parse(&pr_word).unwrap_err().field,
        "pr",
        "the parser refuses it"
    );
    for (op, flags, field) in [
        ("ready", &[][..], "op"),
        // A field the op does not take, or text that is no flag, is never
        // echoed back as a field.
        ("renew", &["--holder", "Ada"][..], "flags"),
        ("renew", &["--task"][..], "task"),
        ("renew", &["task", "a"][..], "flags"),
        ("renew", &["--task", "a", "--task", "b"][..], "task"),
        ("create", &["--depends-on", "a,,b"][..], "depends_on"),
        ("create", &["--depends-on", "a,"][..], "depends_on"),
        ("create", &["--plan", "[]"][..], "flags"),
    ] {
        assert_eq!(line(op, flags).unwrap_err().field, field, "{op} {flags:?}");
    }
    // An op that is none is refused in fixed words, never echoed.
    let stranger = line("<script>alert(1)</script>", &[]).unwrap_err();
    assert_eq!(
        (stranger.field.as_str(), stranger.problem.as_str()),
        ("op", "is not an op of the board")
    );
    let answer = answer::request_refused(None, &stranger);
    assert_eq!(
        answer["error"]["message"],
        json!("op: is not an op of the board")
    );
}

#[test]
fn json_naming_a_key_twice_is_refused() {
    assert_eq!(
        json::strict(r#"{"op": "list_ready"}"#),
        Ok(json!({"op": "list_ready"}))
    );
    for text in [
        r#"{"op": "move", "to": "ready", "to": "done"}"#,
        r#"{"op": "create", "depends_on": ["a"], "depends_on": ["b"]}"#,
        r#"{"op": "x"} trailing"#,
    ] {
        assert!(json::strict(text).is_err(), "{text}");
    }
}

#[test]
fn messages_are_redacted_and_bounded_and_every_code_has_its_one_shape() {
    let leaky = answer::reason_refused(
        Some("bootstrap"),
        answer::FrontEndRefusal::StoreUnavailable,
        &format!("https://user:hunter2@github.com/x {}", "y".repeat(5000)),
    );
    let message = leaky["error"]["message"].as_str().unwrap();
    assert!(!message.contains("hunter2"), "{message}");
    assert!(message.chars().count() <= answer::MAX_MESSAGE_CHARS + 1);
    assert_eq!(leaky["error"]["reason"].as_str().unwrap(), message);
}

/// Every front end's request goes through one entry point that bounds it
/// and reads it strictly before anything else.
#[test]
fn a_request_is_bounded_and_read_strictly_before_it_is_run() {
    let duplicate = read(r#"{"op": "show", "task": "a", "task": "b"}"#).unwrap_err();
    assert_eq!(duplicate.field, "request");
    assert!(duplicate.problem.contains("twice"), "{}", duplicate.problem);
    let padded = format!(
        r#"{{"op": "list"}}{}"#,
        " ".repeat(MAX_REQUEST_BYTES + 1 - 14)
    );
    assert_eq!(padded.len(), MAX_REQUEST_BYTES + 1);
    let big = read(&padded).unwrap_err();
    assert!(big.problem.contains("at most"), "{}", big.problem);
    let fits = format!(r#"{{"op": "list"}}{}"#, " ".repeat(MAX_REQUEST_BYTES - 14));
    assert_eq!(read(&fits), Ok(json!({"op": "list"})));
}

/// A file set aside under a name that looks like a secret is listed as it
/// is, and cleared by that name.
#[test]
fn a_file_whose_name_looks_like_a_secret_is_listed_and_cleared_as_it_is() {
    for path in ["tasks/sk-abcdefgh.json", "tasks/token=x.json"] {
        let store = MemoryBoardStore::default();
        use crate::application::project_board::ports::ProjectBoardStore;
        store.bootstrap().unwrap();
        store.set_aside_path(path, "a stray");
        let operations = assembled(
            Arc::new(store.client()),
            Arc::new(Fixed {
                at: Timestamp::parse("2026-10-10T10:00:00Z").unwrap(),
                referenced: true,
            }),
            Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
            Ok(Arc::new(MemoryPullRequests::default())),
        )
        .unwrap();
        let listed = run(&operations, r#"{"op": "list"}"#);
        let file = &listed["quarantined"][0];
        assert_eq!(file["path"], json!(path), "{listed}");
        let clear =
            json!({"op": "clear_set_aside", "path": file["path"], "object": file["object"]});
        let cleared = run(&operations, &clear.to_string());
        assert_eq!(cleared["ok"], json!(true), "{cleared}");
    }
}
