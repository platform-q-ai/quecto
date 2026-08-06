//! Slice 2 (#1369): container script sets gain `exec` and `kill` operations.

use super::ContainerScriptConfig;

#[test]
fn container_script_config_accepts_exec_argv() {
    let parsed = serde_json::from_str::<ContainerScriptConfig>(
        r#"{"create":["create.sh"],"cleanup":["cleanup.sh"],"exec":["exec.sh","--join"]}"#,
    );
    assert!(
        parsed.is_ok(),
        "script set must accept an exec argv for joining existing environments: {parsed:?}"
    );
}

#[test]
fn container_script_config_accepts_kill_argv() {
    let parsed = serde_json::from_str::<ContainerScriptConfig>(
        r#"{"create":["create.sh"],"cleanup":["cleanup.sh"],"kill":["kill.sh","--force"]}"#,
    );
    assert!(
        parsed.is_ok(),
        "script set must accept a kill argv for explicit environment cleanup: {parsed:?}"
    );
}

#[test]
fn container_script_config_still_rejects_unknown_fields() {
    let parsed = serde_json::from_str::<ContainerScriptConfig>(
        r#"{"create":["create.sh"],"cleanup":["cleanup.sh"],"surprise":["x"]}"#,
    );
    assert!(parsed.is_err(), "unknown script-set fields stay rejected");
}
