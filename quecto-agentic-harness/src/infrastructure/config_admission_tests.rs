use super::*;
use crate::infrastructure::config_admission::default_admission_directory;

fn load(json: &str) -> Result<Config, ConfigError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, json).unwrap();
    Config::load(path.to_str().unwrap())
}

const ENABLED: &str = r#"{"providers":{"openai_compatible":{"endpoints":[{"prefix":"fake","api_key":"k","api_base":"http://127.0.0.1:1","allow_remote_http":true}]}},
"admission":{"directory":"/tmp/x","groups":{"g":{"capacity":2,"reserve":1,"min_interval_ms":5,"queue_capacity":8,"queue_timeout_ms":100,"attempt_timeout_ms":200,"fallback_base_ms":50,"max_cooldown_ms":500}},"aliases":{"acct":"g"},"bindings":{"fake":"acct"},"max_scopes":16,"terminal_capacity":32}}"#;

#[test]
fn absent_admission_section_keeps_the_runtime_disabled() {
    let config = load(r#"{"providers":{"anthropic":{"api_key":"k"}}}"#).unwrap();
    assert!(config.admission_proposal().unwrap().is_none());
}

#[test]
fn enabled_admission_section_yields_a_validated_proposal_and_directory() {
    let config = load(ENABLED).unwrap();
    let (directory, proposal) = config.admission_proposal().unwrap().unwrap();
    assert_eq!(directory, std::path::Path::new("/tmp/x"));
    assert_eq!(proposal.bindings["fake"], "acct");
    let group = crate::domain::inference_admission::GroupId::new("g").unwrap();
    assert_eq!(proposal.policy.groups[&group].capacity, 2);
    assert_eq!(proposal.policy.aliases["acct"], group);
}

#[test]
fn invalid_admission_policy_is_rejected_at_load_time() {
    let invalid = ENABLED.replace("\"reserve\":1", "\"reserve\":2");
    assert!(matches!(load(&invalid), Err(ConfigError::Admission(_))));
    let unknown_alias = ENABLED.replace(
        "\"bindings\":{\"fake\":\"acct\"}",
        "\"bindings\":{\"fake\":\"nope\"}",
    );
    assert!(matches!(
        load(&unknown_alias),
        Err(ConfigError::Admission(_))
    ));
    let unknown_key = ENABLED.replace("\"max_scopes\":16", "\"max_scopes\":16,\"bogus\":1");
    assert!(
        load(&unknown_key).is_err(),
        "unknown admission keys never silently pass"
    );
}

#[test]
fn relative_authority_directory_is_rejected() {
    let relative = ENABLED.replace(
        "\"directory\":\"/tmp/x\"",
        "\"directory\":\"relative/authority\"",
    );
    assert!(matches!(load(&relative), Err(ConfigError::Admission(_))));
}

#[test]
fn admission_proposal_reports_invalid_sections_instead_of_panicking() {
    let mut config = load(ENABLED).unwrap();
    config.admission.as_mut().unwrap().bindings.clear();
    assert!(matches!(
        config.admission_proposal(),
        Err(ConfigError::Admission(_))
    ));
}

#[test]
fn section_errors_name_the_offending_entry() {
    let bad_group = ENABLED
        .replace("\"g\":{\"capacity\"", "\"\":{\"capacity\"")
        .replace("\"acct\":\"g\"", "\"acct\":\"\"");
    assert!(matches!(load(&bad_group), Err(ConfigError::Admission(_))));
    let bad_slot = ENABLED.replace(
        "\"bindings\":{\"fake\":\"acct\"}",
        "\"bindings\":{\"fa/ke\":\"acct\"}",
    );
    let err = load(&bad_slot).unwrap_err().to_string();
    assert!(err.contains("provider slot"), "{err}");
    let no_bindings = ENABLED.replace("\"bindings\":{\"fake\":\"acct\"}", "\"bindings\":{}");
    let err = load(&no_bindings).unwrap_err().to_string();
    assert!(err.contains("binding is required"), "{err}");
}

#[test]
fn default_directory_lives_under_the_base_dir_or_home() {
    assert_eq!(
        default_admission_directory(std::path::Path::new("/srv/quecto")),
        std::path::PathBuf::from("/srv/quecto/admission")
    );
    let fallback = default_admission_directory(std::path::Path::new(""));
    assert!(
        fallback.ends_with(".quecto/admission"),
        "{}",
        fallback.display()
    );
    let without_directory = ENABLED.replace("\"directory\":\"/tmp/x\",", "");
    let config = load(&without_directory)
        .unwrap()
        .with_admission_base_dir(std::path::Path::new("/srv/quecto"));
    let (directory, _) = config.admission_proposal().unwrap().unwrap();
    assert_eq!(directory, std::path::PathBuf::from("/srv/quecto/admission"));
}

#[test]
fn scope_and_terminal_capacities_default_when_omitted() {
    let minimal = ENABLED.replace(",\"max_scopes\":16,\"terminal_capacity\":32", "");
    let config = load(&minimal).unwrap();
    let (_, proposal) = config.admission_proposal().unwrap().unwrap();
    assert_eq!(proposal.policy.max_scopes, 1024);
    assert_eq!(proposal.policy.terminal_capacity, 4096);
}

#[test]
fn authority_directory_may_not_be_the_base_dir_itself() {
    let same = ENABLED.replace("\"directory\":\"/tmp/x\"", "\"directory\":\"/srv/quecto\"");
    let config = load(&same)
        .unwrap()
        .with_admission_base_dir(std::path::Path::new("/srv/quecto"));
    let err = config.admission_proposal().unwrap_err().to_string();
    assert!(err.contains("base directory"), "{err}");
}
