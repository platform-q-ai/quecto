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
    )
    .expect("one credential");
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

    // An idle abort keeps the member (quecto's `handle_abort`); close
    // ends it.
    assert!(!session.abort().await.unwrap().member_ended);
    assert_eq!(session.state().phase, SessionPhase::Idle);
    assert!(session.close().await.unwrap().member_ended);
    tokio::time::timeout(BOUND, async {
        while supervisor.slot_count() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the closed member's process is ended");
}

#[test]
fn the_launch_spec_follows_the_settings() {
    let root = Path::new("/r");
    let none = CredentialEnv {
        name: "CLAUDE_CODE_OAUTH_TOKEN".into(),
        value: String::new(),
    };
    let spec = launch_spec(&settings(root, None), none.clone());
    assert_eq!(spec.model, DEFAULT_CLAUDE_MODEL);
    assert_eq!(spec.member_dir, Path::new("/r/base/claude-members/w1"));
    assert_eq!(spec.checkout, Path::new("/r/checkout"));
    assert_eq!(
        launch_spec(&settings(root, Some("opus")), none).model,
        "opus"
    );
}

#[test]
fn the_credential_is_the_first_one_with_a_value() {
    let oauth = "CLAUDE_CODE_OAUTH_TOKEN";
    let key = "ANTHROPIC_API_KEY";
    let pick = |pairs: &[(&str, &str)]| {
        let credential = credential_from(&environment(pairs)).expect("at most one credential");
        (credential.name, credential.value)
    };
    assert_eq!(pick(&[(oauth, "t")]), (oauth.into(), "t".into()));
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

/// #2287 review (L1): with both credential variables set, which one the
/// member runs under is ambiguous until #2293 selects it by config: the
/// member is refused, naming both variables and neither value.
#[test]
fn both_credentials_set_is_refused_naming_both_and_neither_value() {
    let root = tempfile::tempdir().unwrap();
    let refusal = build_over(
        &settings(root.path(), None),
        environment(&[
            ("ANTHROPIC_API_KEY", "sk-ant-api03-KEYVALUE"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-TOKENVALUE"),
        ]),
        Arc::new(OwnedChildSupervisor::new()),
    )
    .expect_err("both credentials are refused");
    assert_eq!(
        refusal,
        "both CLAUDE_CODE_OAUTH_TOKEN and ANTHROPIC_API_KEY are set; a claude-code member takes \
         exactly one until #2293 selects it by config: unset one"
    );
    assert!(!refusal.contains("VALUE"), "{refusal}");
    // An empty one does not count as set.
    assert!(
        build_over(
            &settings(root.path(), None),
            environment(&[("ANTHROPIC_API_KEY", "k"), ("CLAUDE_CODE_OAUTH_TOKEN", ""),]),
            Arc::new(OwnedChildSupervisor::new()),
        )
        .is_ok()
    );
}

/// #2287 review (M1): a busy abort interrupts the running turn through the
/// real adapter; the stopped turn's result brings the member back to idle,
/// alive, and it takes the next turn.
#[tokio::test]
async fn a_busy_abort_interrupts_the_claude_turn_and_the_member_takes_the_next() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("checkout")).unwrap();
    let scenario = root.path().join("scenario.jsonl");
    std::fs::write(
        &scenario,
        concat!(
            r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":5}"#,
            "\n@await-interrupt\n",
            r#"{"type":"result","subtype":"success","is_error":true,"terminal_reason":"aborted_streaming","user_message_uuids":["@UUID@"]}"#,
            "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"terminal_reason":"completed","result":"again","user_message_uuids":["@UUID@"]}"#,
            "\n",
        ),
    )
    .unwrap();
    let mock = write_mock_claude(&root.path().join("bin"), &scenario);
    let path = format!("{}:/usr/bin:/bin", mock.bin_dir.display());
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let session = build_over(
        &settings(root.path(), None),
        environment(&[("PATH", &path), ("ANTHROPIC_API_KEY", "member-key")]),
        supervisor.clone(),
    )
    .expect("one credential")
    .session;
    session.start().await.expect("the mock starts");
    session.prompt("one", None).await.unwrap();
    let thinking = tokio::time::timeout(BOUND, session.next_step()).await;
    assert!(
        matches!(thinking, Ok(Some(SessionStep::Folded(_)))),
        "{thinking:?}"
    );
    let aborted = session.abort().await.unwrap();
    assert_eq!((aborted.turn, aborted.member_ended), (Some(1), false));
    assert_eq!(
        session.state().phase,
        SessionPhase::Interrupting { turn: 1 }
    );
    let settled = tokio::time::timeout(BOUND, async {
        while session.state().phase != SessionPhase::Idle {
            assert!(session.next_step().await.is_some(), "the member lives on");
        }
    })
    .await;
    assert!(settled.is_ok(), "the stopped turn's result settles it");
    assert_eq!(
        session.prompt("two", None).await,
        Ok(PromptAccepted::Started { turn: 2 })
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
    assert!(ended.is_ok());
    assert_eq!(session.report().unwrap().content, "again");
    let inputs = mock.recorded("input");
    assert_eq!(inputs.len(), 3, "{inputs:?}");
    assert!(inputs[1].contains(r#""subtype":"interrupt""#), "{inputs:?}");
    session.close().await.unwrap();
}
