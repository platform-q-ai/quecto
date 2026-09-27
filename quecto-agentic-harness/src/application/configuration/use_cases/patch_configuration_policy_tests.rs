//! #2247 round 2 L1: a `tools.policy.entries` entry is addressed by its
//! whole stable tool id, and a `set` admits only an id of that grammar.
use super::tests::{GLOBAL, document, patch, unset, use_case};
use super::*;
use crate::application::configuration::use_cases::fakes::{FakeTrust, MemoryStore};
use serde_json::json;

const BASH_ID: &str = "tool.v1:bundled-native:21:quecto:official-tools:bash";

#[test]
fn a_policy_entry_is_set_under_its_whole_stable_id() {
    let store = MemoryStore::with(&[(GLOBAL, "{}")]);
    use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Global,
            GLOBAL,
            &format!("tools.policy.entries.{BASH_ID}"),
            json!({"scope":"parent"}),
        ))
        .unwrap();
    assert_eq!(
        document(&store, GLOBAL),
        json!({"tools":{"policy":{"entries":{BASH_ID:{"scope":"parent"}}}}})
    );
}

/// An entry key that is not a stable id never matches a tool: a `set`
/// refuses it, naming the grammar and a command that works, and writes
/// nothing.
#[test]
fn a_set_refuses_an_entry_that_is_not_a_stable_id() {
    let store = MemoryStore::with(&[(GLOBAL, "{}")]);
    let use_case = use_case(store.clone(), Arc::new(FakeTrust::default()));
    for entry_id in [
        "bash",
        "native:bash",
        "tool.name.v0:bash",
        "tool.v1:bundled-native:21:quecto:official-tools:bash.scope",
        // A zero-padded length is not the id the tool registers under.
        "tool.v1:bundled-native:021:quecto:official-tools:bash",
    ] {
        let key_path = format!("tools.policy.entries.{entry_id}");
        let error = use_case
            .execute(patch(
                ConfigLayer::Global,
                GLOBAL,
                &key_path,
                json!({"scope":"parent"}),
            ))
            .unwrap_err();
        assert_eq!(
            error,
            ConfigPatchError::InvalidPolicyEntryId {
                entry_id: entry_id.to_string()
            }
        );
        let message = error.to_string();
        assert!(message.contains(&format!("`{entry_id}`")), "{message}");
        assert!(
            message.contains("tool.v1:<source>:<length>:<provider>:<name>"),
            "{message}"
        );
        assert!(
            message.contains(
                r#"quecto config set tools.policy.entries.<stable-id> '{"scope":"parent"}'"#
            ),
            "{message}"
        );
    }
    assert_eq!(
        store.content(GLOBAL).as_deref(),
        Some("{}"),
        "nothing written"
    );
}

/// A bad entry already in the file can still be removed by its key.
#[test]
fn an_unset_removes_an_entry_whatever_its_key() {
    let store = MemoryStore::with(&[(
        GLOBAL,
        r#"{"tools":{"policy":{"entries":{"native:bash":{"scope":"none"}}}}}"#,
    )]);
    use_case(store.clone(), Arc::new(FakeTrust::default()))
        .unset(unset(
            ConfigLayer::Global,
            "tools.policy.entries.native:bash",
        ))
        .unwrap();
    assert_eq!(
        document(&store, GLOBAL),
        json!({"tools":{"policy":{"entries":{}}}})
    );
}
