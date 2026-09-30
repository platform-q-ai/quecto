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

#[path = "event_log_admission_tests.rs"]
mod admission;
