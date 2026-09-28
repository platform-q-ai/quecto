//! The claude-code member graph (#2287): the session use case over the
//! claude process adapter, proven against the mock `claude` (the real CLI
//! never runs here).

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use super::*;
use crate::application::external_agent::dto::{PromptAccepted, SessionPhase, SessionStep};
use crate::infrastructure::test_support::mock_claude::write_mock_claude;

const BOUND: Duration = Duration::from_secs(20);

const TURN: &str = concat!(
    r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hello"}]}}"#,
    "\n",
    r#"{"type":"result","subtype":"success","is_error":false,"terminal_reason":"completed","result":"hello"}"#,
    "\n",
);

fn settings(root: &Path, model: Option<&str>) -> ClaudeMemberSettings {
    ClaudeMemberSettings {
        member: "w1".into(),
        model: model.map(str::to_string),
        checkout: root.join("checkout"),
        base_dir: root.join("base"),
    }
}

fn environment(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

#[tokio::test]
async fn the_member_session_drives_the_claude_process_through_a_turn() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("checkout")).unwrap();
    let scenario = root.path().join("scenario.jsonl");
    std::fs::write(&scenario, TURN).unwrap();
    let mock = write_mock_claude(&root.path().join("bin"), &scenario);
    let path = format!("{}:/usr/bin:/bin", mock.bin_dir.display());
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let handles = build_over(
        &settings(root.path(), Some("anthropic/claude-haiku-4-5")),
        environment(&[("PATH", &path), ("CLAUDE_CODE_OAUTH_TOKEN", "member-token")]),
        supervisor.clone(),
    );
    let session = handles.session;
    tokio::time::timeout(BOUND, session.start())
        .await
        .expect("start is bounded")
        .expect("the mock starts");
    assert_eq!(
        session.prompt("hi", None).await,
        Ok(PromptAccepted::Started { turn: 1 })
    );
    let ended = tokio::time::timeout(BOUND, async {
        loop {
            match session.next_step().await {
                Some(SessionStep::Folded(step)) if step.turn_end.is_some() => break,
                Some(_) => continue,
                None => panic!("the stream ended before the turn did"),
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the turn ends within the bound");
    assert_eq!(session.report().unwrap().content, "hello");
    assert_eq!(session.state().phase, SessionPhase::Idle);
    assert!(
        root.path().join("base/claude-members/w1/home").is_dir(),
        "the member's state lives under the base directory"
    );
    let args = mock.recorded("arg");
    let model = args.iter().position(|a| a == "--model").expect("--model");
    assert_eq!(args[model + 1], "claude-haiku-4-5");
    assert_eq!(
        mock.recorded("cwd"),
        [root.path().join("checkout").display().to_string()]
    );

    session.abort().await.unwrap();
    tokio::time::timeout(BOUND, async {
        while supervisor.slot_count() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the aborted member's process is ended");
}

#[test]
fn the_launch_spec_follows_the_settings() {
    let root = Path::new("/r");
    let spec = launch_spec(&settings(root, None), credential_from(&[]));
    assert_eq!(spec.model, DEFAULT_CLAUDE_MODEL);
    assert_eq!(spec.member_dir, Path::new("/r/base/claude-members/w1"));
    assert_eq!(spec.checkout, Path::new("/r/checkout"));
    assert_eq!(
        launch_spec(&settings(root, Some("opus")), credential_from(&[])).model,
        "opus"
    );
}

#[test]
fn the_credential_is_the_first_one_with_a_value() {
    let oauth = "CLAUDE_CODE_OAUTH_TOKEN";
    let key = "ANTHROPIC_API_KEY";
    let pick = |pairs: &[(&str, &str)]| {
        let credential = credential_from(&environment(pairs));
        (credential.name, credential.value)
    };
    assert_eq!(
        pick(&[(key, "k"), (oauth, "t")]),
        (oauth.into(), "t".into())
    );
    assert_eq!(pick(&[(key, "k")]), (key.into(), "k".into()));
    assert_eq!(pick(&[(oauth, ""), (key, "k")]), (key.into(), "k".into()));
    // As getenv sees it: the last value of a repeated name.
    assert_eq!(
        pick(&[(oauth, "t"), (oauth, "")]),
        (oauth.into(), String::new())
    );
    // None at all: the first is named, without a value, for the launcher
    // to refuse.
    assert_eq!(pick(&[]), (oauth.into(), String::new()));
}
