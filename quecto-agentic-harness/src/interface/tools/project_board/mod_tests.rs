use super::boards::Fixed;
use super::*;
use crate::application::project_board::operations::query_listings::QueryListings;
use crate::application::project_board::ports::PullRequestState;
use crate::application::project_board::use_cases::QueryBoard;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project_board/operations.json")
}

/// The fixture's requests, run in order against one board, answer exactly
/// what it holds: the answer shapes are a contract with the CLI's and the
/// tool's clients. A case may run `as` another committer, `at` another
/// time, with `no_reference` on the clock, after setting a task aside
/// (`quarantine`) or a pull request merged (`merge`).
/// `QUECTO_BLESS_BOARD_FIXTURES=1` rewrites the file, and fails the run so
/// a blessed file is always looked at before it passes.
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
        let operations = BoardOperations::new(
            Arc::new(client.clone()),
            Arc::new(clock),
            Arc::new(QueryListings(QueryBoard::new(Arc::new(client)))),
            Ok(Arc::new(host.clone())),
        )
        .unwrap();
        let mut done = case.as_object().unwrap().clone();
        done.insert("answer".into(), run(&operations, &case["request"]));
        answered.push(Value::Object(done));
    }
    if std::env::var_os("QUECTO_BLESS_BOARD_FIXTURES").is_some() {
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
        ("renew", &["--holder", "Ada"][..], "holder"),
        ("renew", &["--task"][..], "task"),
        ("renew", &["task", "a"][..], "task"),
        ("renew", &["--task", "a", "--task", "b"][..], "task"),
        ("create", &["--depends-on", "a,,b"][..], "depends_on"),
        ("create", &["--depends-on", "a,"][..], "depends_on"),
        ("create", &["--plan", "not json"][..], "plan"),
        (
            "create",
            &["--plan", r#"[{"title": "a", "title": "b"}]"#][..],
            "plan",
        ),
    ] {
        assert_eq!(line(op, flags).unwrap_err().field, field, "{op} {flags:?}");
    }
}

#[test]
fn json_naming_a_key_twice_is_refused() {
    assert_eq!(
        json::strict(r#"{"op": "list_ready"}"#),
        Ok(json!({"op": "list_ready"}))
    );
    for text in [
        r#"{"op": "move", "to": "ready", "to": "done"}"#,
        r#"{"plan": [{"title": "a", "title": "b"}]}"#,
        r#"{"op": "x"} trailing"#,
    ] {
        assert!(json::strict(text).is_err(), "{text}");
    }
}

#[test]
fn messages_are_redacted_and_bounded_and_every_code_has_its_one_shape() {
    let leaky = answer::reason_refused(
        Some("bootstrap"),
        answer::ErrorCode::StoreUnavailable,
        &format!("https://user:hunter2@github.com/x {}", "y".repeat(5000)),
    );
    let message = leaky["error"]["message"].as_str().unwrap();
    assert!(!message.contains("hunter2"), "{message}");
    assert!(message.chars().count() <= answer::MAX_MESSAGE_CHARS + 1);
    assert_eq!(leaky["error"]["reason"].as_str().unwrap(), message);
}
