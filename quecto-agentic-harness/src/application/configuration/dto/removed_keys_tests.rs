use super::*;

#[test]
fn the_message_names_each_file_and_the_command_for_each_key() {
    let message = removed_keys_message(&[
        RemovedKeysIn {
            path: "/home/u/.quecto/config.json".into(),
            repair: Repair::Unset("--global"),
            keys: vec!["context_collapse_after_tool_calls", "context_mode"],
        },
        RemovedKeysIn {
            path: "/repo/.quecto/config.json".into(),
            repair: Repair::Unset("--local"),
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

/// Review round 2 M1: an untrusted overlay is edited, then trusted.
#[test]
fn an_untrusted_overlay_is_edited_then_trusted() {
    let message = removed_keys_message(&[RemovedKeysIn {
        path: "/repo/.quecto/config.json".into(),
        repair: Repair::EditThenTrust,
        keys: vec!["context_mode"],
    }]);
    assert!(
        message.contains(
            "remove these keys from /repo/.quecto/config.json by editing it, then run `quecto config trust`"
        ),
        "{message}"
    );
    assert!(
        message.contains("agents.defaults.context_mode"),
        "{message}"
    );
    assert!(!message.contains("quecto config unset"), "{message}");
}
