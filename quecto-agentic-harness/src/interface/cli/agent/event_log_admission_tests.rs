//! #2313 review L5: the event-log switch decided before admission is the
//! build's own, over the same fixtures (a trusted overlay, a `QUECTO_*`
//! override, the global config, and off), and the admission's board calls
//! reach the session's event log once it is attached.
use std::collections::HashMap;
use std::path::Path;

use super::super::{attach_to, decided_with};
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
    let cases: [(&str, bool, Option<bool>, &[(&str, &str)], Option<bool>); 6] = [
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
    attach_to(
        Some(&board),
        &mut agent,
        base.path(),
        &flags,
        "cli:admitted",
        build.event_log,
        &mut stderr,
    );
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:admitted")).unwrap();
    let ops: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["event"] == "swarm_op")
        .collect();
    let names: Vec<&str> = ops.iter().filter_map(|op| op["op"].as_str()).collect();
    assert!(names.contains(&"_bootstrap"), "{names:?}");
    assert_eq!(names[0], "_status", "the admission's first call: {names:?}");
    assert!(
        ops.iter().all(|op| op["lock_wait_us"].is_u64()),
        "measured while held: {text}"
    );
}
