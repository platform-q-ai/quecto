use super::*;

/// #2150: which agents keep an audit log. A workflow session with a key
/// keeps one as before; with the event log on every agent does, keyed by its
/// session or a key of its own, except one asked to leave nothing behind.
#[test]
fn every_agent_but_an_ephemeral_one_logs_when_the_event_log_is_on() {
    assert_eq!(log_key(true, false, "s", false), Some("s".into()));
    assert_eq!(log_key(true, true, "s", false), None);
    assert_eq!(log_key(false, false, "s", false), None);
    assert_eq!(log_key(false, false, "s", true), Some("s".into()));
    assert_eq!(log_key(false, true, "s", true), None);
    assert_eq!(log_key(false, true, "", true), None);
    assert_eq!(log_key(true, false, "", false), None);
    let own = log_key(false, false, "", true).unwrap();
    assert!(
        own.starts_with(&format!("unkeyed-{}-", std::process::id())),
        "{own}"
    );
}

/// #2150 review: a one-shot session logs under the key it runs as.
#[test]
fn a_one_shot_session_logs_under_the_key_it_runs_as() {
    assert_eq!(one_shot_key(None), "cli:default");
    assert_eq!(one_shot_key(Some("foo")), "cli:foo");
}

/// The context and flags of an agent started in `cwd` over `base`, with
/// `config` as its `--config` when given.
fn started(
    base: &Path,
    cwd: &Path,
    config: Option<&Path>,
) -> (crate::interface::cli::CliContext, super::super::AgentFlags) {
    let ctx = crate::interface::cli::CliContext {
        base_dir: Some(base.to_path_buf()),
        cwd: Some(cwd.to_path_buf()),
        config_path: config.map(Path::to_path_buf),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        ..Default::default()
    };
    let args = ["-m".to_string(), "hi".to_string()];
    let mut flags = super::super::parse_agent_flags(&args, &mut String::new()).unwrap();
    flags.adopt_context(&ctx);
    (ctx, flags)
}

const ON: &str = r#"{"telemetry": {"event_log": {"enabled": true}}}"#;

/// #2313: the event-log switch is decided before admission from the
/// configuration the build loads: off by default, on from the global
/// config or an explicit `--config`, and never from an overlay not yet
/// trusted (the build would not apply it either).
#[test]
fn the_event_log_is_decided_before_admission_from_the_same_config() {
    let (base, cwd) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (ctx, flags) = started(base.path(), cwd.path(), None);
    assert!(!decided_before_admission(&ctx, &flags), "off by default");

    std::fs::create_dir_all(cwd.path().join(".quecto")).unwrap();
    std::fs::write(cwd.path().join(".quecto/config.json"), ON).unwrap();
    let (ctx, flags) = started(base.path(), cwd.path(), None);
    assert!(
        !decided_before_admission(&ctx, &flags),
        "an untrusted overlay is not applied"
    );

    let explicit = base.path().join("explicit.json");
    std::fs::write(&explicit, ON).unwrap();
    let (ctx, flags) = started(base.path(), cwd.path(), Some(&explicit));
    assert!(decided_before_admission(&ctx, &flags), "--config");

    std::fs::write(base.path().join("config.json"), ON).unwrap();
    let (ctx, flags) = started(base.path(), cwd.path(), None);
    assert!(decided_before_admission(&ctx, &flags), "the global config");
}

/// Without composition's configuration builder nothing loads: off.
#[test]
fn without_a_configuration_the_event_log_is_off_before_admission() {
    let (base, cwd) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (ctx, mut flags) = started(base.path(), cwd.path(), None);
    flags.configuration = None;
    assert!(!decided_before_admission(&ctx, &flags));
}

/// Epic #2481 T3 review round 1: through `followed`, what
/// `follow_session` does once the crash target gave the arriving
/// session's log (the target itself is armed once per process, so it is
/// not armed here): the board tool's records move to the arriving log,
/// with the switch its session decided, whatever the config files say
/// now; once stopped, a switch starts nothing.
#[tokio::test]
async fn the_board_tools_records_follow_the_session() {
    use crate::application::tools::ports::Tool;
    use crate::infrastructure::persistence::audit_log::AuditLog;
    use crate::interface::tools::project_board::site::BoardSite;
    use crate::interface::tools::project_board::tool::BoardTool;
    let base = tempfile::tempdir().unwrap();
    let logs = ["departing", "arriving", "later"]
        .map(|key| AuditLog::open_sync(base.path(), key).unwrap());
    let [departing, arriving, later] = logs;
    let recordings = Arc::new(SessionRecording::default());
    let board_tool = BoardToolSession::new(Arc::clone(&recordings));
    let build = BoardToolRecordingBuilder(
        crate::composition::bootstrap::project_board::session_board_recording_decided,
    );
    // No config file switches the log on: the session decided it was.
    board_tool.bind(Some(build), true, &departing);
    let site = BoardSite {
        repo: ".".into(),
        remote: "origin".into(),
        build: None,
    };
    let tool = BoardTool::recorded_in(site, recordings);
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    followed(arriving, None, &board_tool);
    tool.execute(r#"{"op": "show", "task": "a"}"#)
        .await
        .unwrap();
    board_tool.stop();
    followed(later, None, &board_tool);
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    let ops = |key: &str| -> Vec<serde_json::Value> {
        let text =
            std::fs::read_to_string(AuditLog::file_path(base.path(), key)).unwrap_or_default();
        text.lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|line| line["event"] == "board_op")
            .map(|line| line["op"].clone())
            .collect()
    };
    assert_eq!(ops("departing"), ["list"]);
    assert_eq!(ops("arriving"), ["show"]);
    assert_eq!(ops("later"), Vec::<serde_json::Value>::new());
}

#[path = "event_log_admission_tests.rs"]
mod admission;
