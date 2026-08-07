//! Slice 2 (#1369): container configs gain `exec` and `kill` operations.

use super::ContainerConfig;

#[test]
fn container_config_accepts_exec_argv() {
    let parsed = serde_json::from_str::<ContainerConfig>(
        r#"{"create":["create.sh"],"cleanup":["cleanup.sh"],"exec":["exec.sh","--join"]}"#,
    );
    assert!(
        parsed.is_ok(),
        "script set must accept an exec argv for joining existing environments: {parsed:?}"
    );
}

#[test]
fn container_config_accepts_kill_argv() {
    let parsed = serde_json::from_str::<ContainerConfig>(
        r#"{"create":["create.sh"],"cleanup":["cleanup.sh"],"kill":["kill.sh","--force"]}"#,
    );
    assert!(
        parsed.is_ok(),
        "script set must accept a kill argv for explicit environment cleanup: {parsed:?}"
    );
}

#[test]
fn container_config_still_rejects_unknown_fields() {
    let parsed = serde_json::from_str::<ContainerConfig>(
        r#"{"create":["create.sh"],"cleanup":["cleanup.sh"],"surprise":["x"]}"#,
    );
    assert!(parsed.is_err(), "unknown script-set fields stay rejected");
}

#[test]
fn container_configs_require_exactly_one_default_label() {
    // #1410: default is an entry label validated at LOAD time, not spawn time.
    let none = r#"{"container_configs":{"a":{"create":["c"],"cleanup":["k"]}}}"#;
    let err = serde_json::from_str::<super::Config>(none)
        .map_err(|e| e.to_string())
        .and_then(|c| {
            c.validate_container_configs_for_test()
                .map_err(|e| e.to_string())
        })
        .unwrap_err();
    assert!(err.contains("no container config is labeled"), "{err}");
    assert!(
        err.contains("a"),
        "error must enumerate configured names: {err}"
    );

    let two = r#"{"container_configs":{
        "a":{"default":true,"create":["c"],"cleanup":["k"]},
        "b":{"default":true,"create":["c"],"cleanup":["k"]}}}"#;
    let err = serde_json::from_str::<super::Config>(two)
        .map_err(|e| e.to_string())
        .and_then(|c| {
            c.validate_container_configs_for_test()
                .map_err(|e| e.to_string())
        })
        .unwrap_err();
    assert!(err.contains("multiple container configs"), "{err}");

    let one = r#"{"container_configs":{
        "a":{"default":true,"create":["c"],"cleanup":["k"]},
        "b":{"create":["c"],"cleanup":["k"]}}}"#;
    let cfg = serde_json::from_str::<super::Config>(one).unwrap();
    assert!(cfg.validate_container_configs_for_test().is_ok());
    // Empty maps stay valid: containers are optional.
    let empty = serde_json::from_str::<super::Config>("{}").unwrap();
    assert!(empty.validate_container_configs_for_test().is_ok());
}
