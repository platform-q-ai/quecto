//! Contract for the `RuntimeConfigurationSource` port (#1849), against the
//! file-backed adapter: a change is reported once per edit of the config
//! file or `models.json`; a rebuild yields the provider and the persisted
//! tool policy from one read of the current files, or the parse error; and
//! a rebuild observes the files before reading them, so a forced rebuild
//! never prompts a second one at the next poll (fix (b)).
use std::collections::HashMap;

use quecto::application::catalogue::ports::RuntimeConfigurationSource;
use quecto::domain::tool_descriptor::ProfileAvailabilityScope;
use quecto::infrastructure::runtime_configuration::FileRuntimeConfiguration;

const OPENAI_ONLY: &str =
    r#"{"providers":{"openai":{"api_key":"sk-test","api_base":"http://127.0.0.1:9"}}}"#;

fn with_policy(scope: &str) -> String {
    format!(
        r#"{{"providers":{{"openai":{{"api_key":"sk-test","api_base":"http://127.0.0.1:9"}}}},
            "tools":{{"policy":{{"entries":{{"native:bash":{{"scope":"{scope}"}}}}}}}}}}"#
    )
}

/// Rewrite `path` so the mtime is guaranteed to differ from the current one.
fn edit(path: &std::path::Path, content: &str) {
    let before = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    loop {
        std::fs::write(path, content).unwrap();
        let after = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if after != before {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

fn under_test(dir: &std::path::Path) -> (Box<dyn RuntimeConfigurationSource>, std::path::PathBuf) {
    let config_path = dir.join("config.json");
    let source = FileRuntimeConfiguration::seeded(
        config_path.clone(),
        dir.to_path_buf(),
        HashMap::new(),
        reqwest::Client::new(),
        quecto::composition::runtime::build_agent_provider,
    );
    (Box::new(source), config_path)
}

#[test]
fn a_change_is_reported_once_per_edit_of_either_watched_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), OPENAI_ONLY).unwrap();
    let (mut source, config_path) = under_test(dir.path());
    assert!(!source.changed(), "seeded from the current files");

    edit(&config_path, &with_policy("none"));
    assert!(source.changed());
    assert!(!source.changed(), "an edit is reported once");

    edit(&dir.path().join("models.json"), r#"{"providers":{}}"#);
    assert!(source.changed(), "models.json is watched too");
    assert!(!source.changed());
}

#[test]
fn a_rebuild_yields_provider_and_tool_policy_from_one_read_of_the_current_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), with_policy("child")).unwrap();
    let (mut source, _) = under_test(dir.path());
    let rebuilt = source.rebuild().expect("a valid config rebuilds");
    assert!(!rebuilt.provider.name().is_empty());
    assert_eq!(
        rebuilt.tool_policy.get("native:bash"),
        Some(&ProfileAvailabilityScope::Child)
    );
}

#[test]
fn a_malformed_config_is_a_rebuild_error_naming_the_parse_failure() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), "{ invalid json").unwrap();
    let (mut source, _) = under_test(dir.path());
    let error = source
        .rebuild()
        .expect_err("malformed config cannot rebuild");
    assert!(!error.is_empty());
}

/// Fix (b): a forced rebuild consumes the pending change — the poll after
/// it does not rebuild the same files again.
#[test]
fn a_rebuild_observes_the_files_so_the_next_poll_reports_no_change() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), OPENAI_ONLY).unwrap();
    let (mut source, config_path) = under_test(dir.path());
    edit(&config_path, &with_policy("none"));
    source.rebuild().expect("rebuild without polling first");
    assert!(
        !source.changed(),
        "the forced rebuild already read this edit; nothing to rebuild again"
    );
    edit(&config_path, &with_policy("child"));
    assert!(source.changed(), "a later edit is still detected");
}
