use super::*;
use crate::infrastructure::config::Config;

fn load(extensions: serde_json::Value) -> Result<Config, ConfigError> {
    Config::from_document(serde_json::json!({ "extensions": extensions }))
}

#[test]
fn an_entry_reads_with_children_on_by_default() {
    let config = load(serde_json::json!([{
        "name": "browser-task",
        "command": "/opt/bt",
        "args": ["--socket", "{socket}"],
        "env": {"PROFILE": "{state_dir}/p"},
    }]))
    .unwrap();
    let spec = config.extensions[0].spec();
    assert!(spec.children);
    assert_eq!(spec.args, ["--socket", "{socket}"]);
    assert_eq!(spec.env["PROFILE"], "{state_dir}/p");
    assert!(Config::default().extensions.is_empty());
}

#[test]
fn a_bad_entry_refuses_the_config_naming_why() {
    for (entry, says) in [
        (
            serde_json::json!({"name": "x", "command": "bt"}),
            "absolute path",
        ),
        (
            serde_json::json!({"name": "x y", "command": "/bt"}),
            "letters",
        ),
        (serde_json::json!({"name": "", "command": "/bt"}), "letters"),
        (
            serde_json::json!({"name": "x", "command": "/bt", "args": ["{sock}"]}),
            "{sock}",
        ),
        (
            serde_json::json!({"name": "x", "command": "/bt", "env": {"A": "{home}"}}),
            "{home}",
        ),
        (
            serde_json::json!({"name": "x", "command": "/bt", "env": {"A=B": "v"}}),
            "env name",
        ),
    ] {
        let error = load(serde_json::json!([entry])).unwrap_err().to_string();
        assert!(
            error.contains("invalid extensions") && error.contains(says),
            "{error}"
        );
    }
    let twice = serde_json::json!([
        {"name": "x", "command": "/bt"},
        {"name": "x", "command": "/other"},
    ]);
    assert!(
        load(twice)
            .unwrap_err()
            .to_string()
            .contains("more than once")
    );
    let typo = serde_json::json!([{"name": "x", "command": "/bt", "child": false}]);
    assert!(load(typo).unwrap_err().to_string().contains("child"));
}
