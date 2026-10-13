//! #2313 review L5: the event-log switch decided before admission is the
//! build's own, over the same fixtures (a trusted overlay, a `QUECTO_*`
//! override, the global config, and off), and the admission's board calls
//! reach the session's event log once it is attached.
use std::collections::HashMap;
use std::path::Path;

use super::super::{BoardToolSession, SessionLogTarget, SessionRecording, attach_to, decided_with};
use crate::application::configuration::ports::OverlayTrustStore;
use crate::infrastructure::config::persistence::PersistentOverlayTrustStore;
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use crate::interface::cli::CliContext;
use crate::interface::cli::agent::{AgentFlags, build_agent_from_config_in, swarm_runtime};

const PROVIDERS: &str = r#""providers":{"fireworks":{"api_key":"k"}}"#;
const ON: &str = r#""telemetry":{"event_log":{"enabled":true}}"#;

/// A base directory whose global config names a provider, with the event
/// log switched on there when `global_on`, and its models.
fn base(global_on: bool) -> tempfile::TempDir {
    let base = tempfile::tempdir().unwrap();
    let config = match global_on {
        true => format!("{{{PROVIDERS},{ON}}}"),
        false => format!("{{{PROVIDERS}}}"),
    };
    std::fs::write(base.path().join("config.json"), config).unwrap();
    std::fs::write(
        base.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"some-model","maxTokens":8192}]}}}"#,
    )
    .unwrap();
    base
}

/// The overlay switching the event log on in `cwd`, trusted in `base`
/// when `trusted`.
fn overlay(base: &Path, cwd: &Path, trusted: bool) {
    let path = cwd.join(".quecto/config.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let content = format!("{{{ON}}}");
    std::fs::write(&path, &content).unwrap();
    if trusted {
        PersistentOverlayTrustStore::for_base_dir(base, false)
            .approve(&path, content.as_bytes())
            .unwrap();
    }
}

/// An agent started in `cwd` over `base`, composed as `main` composes it.
fn started(base: &Path, cwd: &Path) -> (CliContext, AgentFlags) {
    let ctx = CliContext {
        base_dir: Some(base.to_path_buf()),
        cwd: Some(cwd.to_path_buf()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        swarm_board: Some(crate::composition::swarm::build_swarm_board_handles),
        swarm_board_wire: Some(crate::composition::swarm::board_wire()),
        swarm_board_log: Some(crate::composition::swarm::board_op_log),
        ..Default::default()
    };
    let args = ["-m".to_string(), "hi".to_string()];
    let mut flags = super::super::super::parse_agent_flags(&args, &mut String::new()).unwrap();
    flags.adopt_context(&ctx);
    flags.model_override = Some("fireworks/some-model".into());
    (ctx, flags)
}

/// The build's event-log switch, `None` when the build refused to start.
fn built(ctx: &CliContext, flags: &AgentFlags, env: &HashMap<String, String>) -> Option<bool> {
    let selection = ctx.config_selection().unwrap();
    let mut stderr = String::new();
    build_agent_from_config_in(&ctx.base_dir(), &selection, flags, &mut stderr, None, env)
        .map(|build| build.event_log)
}

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// Over each fixture the switch decided before admission is the one the
/// build then takes; where the build refuses (an override it rejects), no
/// agent starts, and the decision is off too.
#[test]
fn the_switch_decided_before_admission_is_the_builds() {
    // (case, the global config switches the log on, an overlay switching
    // it on is present and trusted, the QUECTO_* overrides, the build's).
    type Case<'a> = (
        &'a str,
        bool,
        Option<bool>,
        &'a [(&'a str, &'a str)],
        Option<bool>,
    );
    let cases: [Case<'_>; 6] = [
        ("off", false, None, &[], Some(false)),
        ("a trusted overlay", false, Some(true), &[], Some(true)),
        ("an untrusted overlay", false, Some(false), &[], Some(false)),
        ("the global config", true, None, &[], Some(true)),
        (
            "a QUECTO_* override the build applies",
            true,
            None,
            &[("QUECTO_AGENTS_DEFAULTS_MAX_TOKENS", "4096")],
            Some(true),
        ),
        (
            "a QUECTO_* override the build rejects",
            false,
            Some(true),
            &[("QUECTO_AGENTS_DEFAULTS_EFFORT", "bogus")],
            None,
        ),
    ];
    for (case, global_on, trusted, overrides, expected) in cases {
        let base = base(global_on);
        let cwd = tempfile::tempdir().unwrap();
        if let Some(trusted) = trusted {
            overlay(base.path(), cwd.path(), trusted);
        }
        let (ctx, flags) = started(base.path(), cwd.path());
        let overrides = env(overrides);
        let build = built(&ctx, &flags, &overrides);
        assert_eq!(build, expected, "{case}: the build");
        assert_eq!(
            decided_with(&ctx, &flags, &overrides),
            build.unwrap_or(false),
            "{case}: decided before admission"
        );
    }
}

/// End to end: a creator admitted with the event log on holds its
/// admission's board calls, and once the build's log is attached they are
/// the first `swarm_op` lines of the session's file, measured.
#[test]
fn the_admissions_calls_are_in_the_file_once_the_log_is_attached() {
    let base = base(true);
    let checkout = tempfile::tempdir().unwrap();
    std::fs::create_dir(checkout.path().join(".git")).unwrap();
    let (ctx, mut flags) = started(base.path(), checkout.path());
    let board = swarm_runtime::admission_board(&ctx, &flags, true)
        .unwrap()
        .expect("composition's board");
    let creator = SwarmContext {
        board: board.clone(),
        checkout: checkout.path().to_path_buf(),
        member: "creator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    let mut stderr = String::new();
    assert!(
        swarm_runtime::admit_with(Some(creator), true, &mut flags, &mut stderr),
        "{stderr}"
    );
    let selection = ctx.config_selection().unwrap();
    let build = build_agent_from_config_in(
        &ctx.base_dir(),
        &selection,
        &flags,
        &mut stderr,
        None,
        &HashMap::new(),
    )
    .unwrap_or_else(|| panic!("{stderr}"));
    assert!(build.event_log);
    let mut agent = build.agent;
    let target = SessionLogTarget {
        base_dir: base.path(),
        session_key: "cli:admitted",
        event_log: build.event_log,
        project_board: None,
    };
    let board_tool = BoardToolSession::new(std::sync::Arc::default());
    attach_to(
        Some(&board),
        &board_tool,
        &mut agent,
        &target,
        &flags,
        &mut stderr,
    );
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:admitted")).unwrap();
    let ops: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["event"] == "swarm_op")
        .collect();
    let names: Vec<&str> = ops.iter().filter_map(|op| op["op"].as_str()).collect();
    // The creator's admission: no store yet (so no `_status` read), then
    // its bootstrap, which creates the store and joins it.
    assert_eq!(
        names.first(),
        Some(&"_bootstrap"),
        "the admission's call, first: {names:?}"
    );
    assert!(
        ops.iter().all(|op| op["lock_wait_us"].is_u64()),
        "measured while held: {text}"
    );
}

/// #2414: an agent whose environment still sets the removed context-mode
/// override is refused at build, the override named.
#[test]
fn an_agent_whose_environment_sets_the_removed_mode_is_refused() {
    let base = base(false);
    let cwd = tempfile::tempdir().unwrap();
    let (ctx, mut flags) = started(base.path(), cwd.path());
    flags.model_override = Some("fireworks/some-model".into());
    let selection = ctx.config_selection().unwrap();
    let mut stderr = String::new();
    let built = build_agent_from_config_in(
        &ctx.base_dir(),
        &selection,
        &flags,
        &mut stderr,
        None,
        &env(&[("QUECTO_CONTEXT_MODE", "watermark")]),
    );
    assert!(built.is_none(), "refused");
    assert!(
        stderr.contains("QUECTO_CONTEXT_MODE") && stderr.contains("removed"),
        "{stderr}"
    );
}

/// Epic #2481 T3 review round 1: a session attached with the event log on
/// and composition's builder binds the board tool's recording to its log,
/// so a tool call lands there; with no builder, nothing does.
#[tokio::test(flavor = "multi_thread")]
async fn an_attached_session_records_the_board_tools_calls() {
    use crate::application::tools::ports::Tool;
    use crate::interface::cli::board::BoardToolRecordingBuilder;
    use crate::interface::tools::project_board::site::BoardSite;
    use crate::interface::tools::project_board::tool::BoardTool;
    let base = base(true);
    let cwd = tempfile::tempdir().unwrap();
    let (ctx, flags) = started(base.path(), cwd.path());
    let selection = ctx.config_selection().unwrap();
    for (session, builder, recorded) in [
        (
            "cli:board-on",
            Some(BoardToolRecordingBuilder(
                crate::composition::bootstrap::project_board::session_board_recording_decided,
            )),
            1,
        ),
        ("cli:board-unbuilt", None, 0),
    ] {
        let mut stderr = String::new();
        let build = build_agent_from_config_in(
            &ctx.base_dir(),
            &selection,
            &flags,
            &mut stderr,
            None,
            &HashMap::new(),
        )
        .unwrap_or_else(|| panic!("{stderr}"));
        assert!(build.event_log);
        let mut agent = build.agent;
        let recordings = std::sync::Arc::new(SessionRecording::default());
        let board_tool = BoardToolSession::new(std::sync::Arc::clone(&recordings));
        let target = SessionLogTarget {
            base_dir: base.path(),
            session_key: session,
            event_log: true,
            project_board: builder,
        };
        attach_to(None, &board_tool, &mut agent, &target, &flags, &mut stderr);
        let site = BoardSite {
            repo: cwd.path().into(),
            remote: "origin".into(),
            build: None,
        };
        BoardTool::recorded_in(site, recordings)
            .execute(r#"{"op": "list"}"#)
            .await
            .unwrap();
        let text =
            std::fs::read_to_string(AuditLog::file_path(base.path(), session)).unwrap_or_default();
        let ops = text
            .lines()
            .filter(|line| line.contains(r#""event":"board_op""#))
            .count();
        assert_eq!(ops, recorded, "{session}");
    }
}

/// Review round 2: off means off where it is wired in. A session whose
/// event log is off keeps its workflow audit log; given composition's
/// builder, its board tool records nothing there, nor after a switch.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_the_event_log_off_records_no_board_tool_call() {
    use crate::application::tools::ports::Tool;
    use crate::interface::cli::board::BoardToolRecordingBuilder;
    use crate::interface::tools::project_board::site::BoardSite;
    use crate::interface::tools::project_board::tool::BoardTool;
    let base = base(false);
    let cwd = tempfile::tempdir().unwrap();
    let (ctx, mut flags) = started(base.path(), cwd.path());
    let selection = ctx.config_selection().unwrap();
    let mut stderr = String::new();
    let build = build_agent_from_config_in(
        &ctx.base_dir(),
        &selection,
        &flags,
        &mut stderr,
        None,
        &HashMap::new(),
    )
    .unwrap_or_else(|| panic!("{stderr}"));
    assert!(!build.event_log);
    let mut agent = build.agent;
    // A workflow session keeps its audit log with the event log off.
    flags.workflow = true;
    let recordings = std::sync::Arc::new(SessionRecording::default());
    let board_tool = BoardToolSession::new(std::sync::Arc::clone(&recordings));
    let target = SessionLogTarget {
        base_dir: base.path(),
        session_key: "cli:workflow-off",
        event_log: false,
        project_board: Some(BoardToolRecordingBuilder(
            crate::composition::bootstrap::project_board::session_board_recording_decided,
        )),
    };
    attach_to(None, &board_tool, &mut agent, &target, &flags, &mut stderr);
    assert!(
        AuditLog::file_path(base.path(), "cli:workflow-off").exists(),
        "the workflow log is open"
    );
    let site = BoardSite {
        repo: cwd.path().into(),
        remote: "origin".into(),
        build: None,
    };
    let tool = BoardTool::recorded_in(site, recordings);
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    let arriving = AuditLog::open_sync(base.path(), "cli:workflow-next").unwrap();
    super::super::followed(arriving, None, &board_tool);
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    for session in ["cli:workflow-off", "cli:workflow-next"] {
        let text =
            std::fs::read_to_string(AuditLog::file_path(base.path(), session)).unwrap_or_default();
        assert!(!text.contains(r#""event":"board_op""#), "{session}: {text}");
    }
}
