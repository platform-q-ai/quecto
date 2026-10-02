use super::*;
use serde_json::json;

fn object(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn every_removed_key_set_is_found_in_order_null_included() {
    let document = object(json!({"agents": {"defaults": {
        "context_collapse_after_messages": 100,
        "model": "m",
        "context_mode": null,
    }}}));
    assert_eq!(
        removed_keys_set(&document),
        ["context_mode", "context_collapse_after_messages"]
    );
    assert!(removed_keys_set(&object(json!({"agents": 3}))).is_empty());
    let stripped = without_removed_keys(&document);
    assert_eq!(
        stripped,
        object(json!({"agents": {"defaults": {"model": "m"}}}))
    );
}

#[test]
fn the_message_names_each_file_and_the_command_for_each_key() {
    let message = removed_keys_message(&[
        RemovedKeysIn {
            path: "/home/u/.quecto/config.json".into(),
            flag: "--global",
            keys: vec!["context_collapse_after_tool_calls", "context_mode"],
        },
        RemovedKeysIn {
            path: "/repo/.quecto/config.json".into(),
            flag: "--local",
            keys: vec!["context_collapse_after_messages"],
        },
    ]);
    for line in [
        "/home/u/.quecto/config.json:",
        "quecto config unset agents.defaults.context_collapse_after_tool_calls --global",
        "quecto config unset agents.defaults.context_mode --global",
        "/repo/.quecto/config.json:",
        "quecto config unset agents.defaults.context_collapse_after_messages --local",
        "#2414",
    ] {
        assert!(message.contains(line), "{line:?} in {message}");
    }
}
