use super::*;

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
