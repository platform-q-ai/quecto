//! A claude-code member's event log (#2304), proven over the mock `claude`
//! (the real CLI never runs here): each spike capture replayed through the
//! session files a pinned sequence of events; nothing is written unless
//! `telemetry.event_log` is on; no credential, proxy value, assistant text
//! or thinking reaches the log.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::application::external_agent::dto::SessionStep;
use crate::infrastructure::test_support::mock_claude::write_mock_claude;

const BOUND: Duration = Duration::from_secs(20);
const PARENT: &str = "cli:coordinator";
const CREDENTIAL: &str = "sk-ant-oat01-CREDENTIALSECRETCREDENTIALSECRET";
const PROXY: &str = "http://proxyuser:PROXYSECRETPASS@proxy.internal:3128";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude_code")
}

fn settings(root: &Path, config_path: Option<PathBuf>) -> ClaudeMemberSettings {
    ClaudeMemberSettings {
        member: "w1".into(),
        model: None,
        checkout: root.join("checkout"),
        base_dir: root.join("base"),
        parent: Some(PARENT.into()),
        config_path,
    }
}

fn switch_on(config: &Path, enabled: bool) {
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    let json = serde_json::json!({"telemetry": {"event_log": {"enabled": enabled}}});
    std::fs::write(config, json.to_string()).unwrap();
}

/// Run a member over `scenario` for as many turns as it has results, then
/// close it; answers the event log's lines, if it wrote one.
async fn run_member(root: &Path, scenario: &Path, config_path: Option<PathBuf>) -> Vec<String> {
    std::fs::create_dir_all(root.join("checkout")).unwrap();
    let turns = std::fs::read_to_string(scenario)
        .unwrap()
        .lines()
        .filter(|line| line.contains(r#""type": "result""#) || line.contains(r#""type":"result""#))
        .count();
    let mock = write_mock_claude(&root.join("bin"), scenario);
    let path = format!("{}:/usr/bin:/bin", mock.bin_dir.display());
    let environment: Vec<(OsString, OsString)> = [
        ("PATH", path.as_str()),
        ("CLAUDE_CODE_OAUTH_TOKEN", CREDENTIAL),
        ("HTTPS_PROXY", PROXY),
        ("QUECTO_SWARM_MEMBER", "C3"),
    ]
    .iter()
    .map(|(name, value)| (OsString::from(name), OsString::from(value)))
    .collect();
    let handles = build_over(
        &settings(root, config_path),
        environment,
        Arc::new(OwnedChildSupervisor::new()),
    )
    .expect("one credential");
    let session = handles.session.clone();
    tokio::time::timeout(BOUND, session.start())
        .await
        .expect("start is bounded")
        .expect("the mock starts");
    for turn in 0..turns {
        session.prompt(&format!("turn {turn}"), None).await.unwrap();
        tokio::time::timeout(BOUND, async {
            loop {
                match session.next_step().await {
                    Some(SessionStep::Folded(step)) if step.turn_end.is_some() => break,
                    Some(_) => continue,
                    None => panic!("the stream ended before the turn did"),
                }
            }
        })
        .await
        .expect("the turn ends within the bound");
    }
    session.close().await.unwrap();
    // As the runner does: the event log keeps what it holds.
    session.finish().await;
    drop(session);
    drop(handles);
    let log = AuditLog::file_path(&root.join("base"), "cli:w1");
    std::fs::read_to_string(log)
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// A line with what varies from run to run taken out: the envelope's
/// time, process and host, and the durations the session's clock measured.
fn normalized(line: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
    let object = value.as_object_mut().unwrap();
    for varying in ["ts", "unix_ms", "pid", "host"] {
        object.remove(varying);
    }
    match object["event"].as_str() {
        Some("external_agent_tool") => {
            object.insert("duration_ms".into(), 0.into());
        }
        Some("external_agent_lifecycle") if object.contains_key("wall_ms") => {
            object.insert("wall_ms".into(), 0.into());
        }
        _ => {}
    }
    // Keys sorted: a golden line reads the same whatever order they were
    // written in.
    let sorted: std::collections::BTreeMap<&String, &serde_json::Value> = object.iter().collect();
    serde_json::to_string(&sorted).unwrap()
}

async fn golden(name: &str) {
    let root = tempfile::tempdir().unwrap();
    switch_on(&root.path().join("base/config.json"), true);
    let scenario = fixtures().join(format!("{name}.stream.jsonl"));
    let lines = run_member(root.path(), &scenario, None).await;
    let produced: Vec<String> = lines.iter().map(|line| normalized(line)).collect();
    // What the normalisation takes out was there: the member's end, timed.
    let ended: Vec<serde_json::Value> = lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|value| value["kind"] == "ended")
        .collect();
    let [ended] = ended.as_slice() else {
        panic!("{name}: one end is recorded: {lines:?}")
    };
    assert!(ended["wall_ms"].is_u64(), "{name}: {ended}");
    let golden = fixtures().join(format!("telemetry/{name}.events.jsonl"));
    if std::env::var_os("QUECTO_BLESS_GOLDEN").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, produced.join("\n") + "\n").unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    let expected: Vec<&str> = expected.lines().collect();
    assert!(
        !produced.is_empty(),
        "{name}: the member wrote its event log"
    );
    assert_eq!(
        produced, expected,
        "{name}: the event sequence is the golden one"
    );
    for line in &produced {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["session"], "cli:w1", "{line}");
        assert_eq!(value["parent"], PARENT, "{line}");
        assert_eq!(value["member_ref"], "C3", "{line}");
    }
}

#[tokio::test]
async fn the_rt_capture_files_its_golden_events() {
    golden("rt").await;
}

#[tokio::test]
async fn the_guards_capture_files_its_golden_events() {
    golden("guards").await;
}

#[tokio::test]
async fn the_mid_capture_files_its_golden_events() {
    golden("mid").await;
}

#[tokio::test]
async fn the_kill_capture_files_its_golden_events() {
    golden("kill").await;
}

#[tokio::test]
async fn the_errors_capture_files_its_golden_events() {
    golden("errors").await;
}

#[tokio::test]
async fn the_not_logged_in_capture_files_its_golden_events() {
    golden("not_logged_in").await;
}

/// Owner decision T1: the event log is off unless configured; with it
/// off (no config, or a config that leaves it off) nothing is written.
#[tokio::test]
async fn nothing_is_written_unless_the_event_log_is_switched_on() {
    let scenario = fixtures().join("rt.stream.jsonl");
    let root = tempfile::tempdir().unwrap();
    assert!(run_member(root.path(), &scenario, None).await.is_empty());
    let off = root.path().join("off.json");
    switch_on(&off, false);
    let root = tempfile::tempdir().unwrap();
    assert!(
        run_member(root.path(), &scenario, Some(off))
            .await
            .is_empty()
    );
    assert!(
        !root.path().join("base/audit").exists(),
        "not even the log's directory"
    );

    let on = root.path().join("on.json");
    switch_on(&on, true);
    let root = tempfile::tempdir().unwrap();
    assert!(
        !run_member(root.path(), &scenario, Some(on))
            .await
            .is_empty(),
        "the --config given switches it on"
    );
}

const SECRETS_TURN: &str = concat!(
    r#"{"type":"system","subtype":"init","session_id":"s-1","model":"claude-haiku-4-5","claude_code_version":"2.1.280","tools":["Bash"],"mcp_servers":[]}"#,
    "\n",
    r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":"THINKINGWORDS about the plan"}]}}"#,
    "\n",
    r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"ASSISTANTWORDS for the user"}]}}"#,
    "\n",
    r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"curl -H 'Authorization: Bearer sk-ant-api03-BASHSECRETBASHSECRETBASHSECRET' https://api.example"}}]}}"#,
    "\n",
    r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"TOOLOUTPUTWORDS"}]}}"#,
    "\n",
    r#"{"type":"result","subtype":"success","is_error":false,"terminal_reason":"completed","result":"RESULTWORDS","total_cost_usd":0.002,"usage":{"input_tokens":5,"output_tokens":6}}"#,
    "\n",
);

#[tokio::test]
async fn no_credential_proxy_value_or_transcript_reaches_the_log() {
    let root = tempfile::tempdir().unwrap();
    switch_on(&root.path().join("base/config.json"), true);
    let scenario = root.path().join("secrets.jsonl");
    std::fs::write(&scenario, SECRETS_TURN).unwrap();
    let log = run_member(root.path(), &scenario, None).await.join("\n");
    assert!(log.contains(r#""event":"external_agent_tool""#), "{log}");
    assert!(log.contains(r#""event":"external_agent_turn""#), "{log}");
    assert!(log.contains(r#""credential_mode":"oauth_token""#), "{log}");
    for secret in [
        "CREDENTIALSECRET",
        "PROXYSECRETPASS",
        "proxyuser",
        "BASHSECRET",
        "THINKINGWORDS",
        "ASSISTANTWORDS",
        "TOOLOUTPUTWORDS",
        "RESULTWORDS",
        "turn 0",
    ] {
        assert!(!log.contains(secret), "{secret} leaked: {log}");
    }
}

/// The ref a record carries is the swarm's name for the member when it is
/// a recordable name, else the member's own session name (#2304 review).
#[test]
fn the_member_ref_is_the_swarm_s_name_only_when_it_is_a_recordable_name() {
    let environment =
        |value: &str| vec![(OsString::from("QUECTO_SWARM_MEMBER"), OsString::from(value))];
    assert_eq!(member_ref(&environment("C3"), "w1"), "C3");
    for unrecordable in ["", " ", "C 3", "C3/../x", &"x".repeat(65)] {
        assert_eq!(
            member_ref(&environment(unrecordable), "w1"),
            "w1",
            "{unrecordable:?}"
        );
    }
    assert_eq!(member_ref(&[], "w1"), "w1");
}

/// Every credential variable a member can run under has a mode.
#[test]
fn every_credential_variable_names_its_mode_and_never_its_value() {
    let modes: Vec<&str> = CREDENTIAL_VARIABLES
        .iter()
        .map(|name| {
            credential_mode(&CredentialEnv {
                name: name.to_string(),
                value: "secret".into(),
            })
        })
        .collect();
    assert_eq!(modes, ["oauth_token", "api_key"]);
    let unset = CredentialEnv {
        name: CREDENTIAL_VARIABLES[0].to_string(),
        value: String::new(),
    };
    assert_eq!(credential_mode(&unset), "none");
}
