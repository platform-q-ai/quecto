use super::*;
use serde_json::json;

fn merged(global: Value, overlay: Value) -> Value {
    let global = global.as_object().unwrap().clone();
    let overlay = overlay.as_object().unwrap().clone();
    Value::Object(merge_overlay(global, overlay))
}

#[test]
fn agents_defaults_merge_field_wise() {
    let out = merged(
        json!({"agents":{"defaults":{"model":"g","effort":"high","max_tokens":10}}}),
        json!({"agents":{"defaults":{"model":"l"}}}),
    );
    assert_eq!(
        out,
        json!({"agents":{"defaults":{"model":"l","effort":"high","max_tokens":10}}})
    );
}

#[test]
fn tools_web_engines_and_policy_entries_merge_entry_wise() {
    let out = merged(
        json!({"tools":{"web":{"brave":{"enabled":true,"max_results":9},"fetch":{"enabled":false}},
                        "policy":{"entries":{"exec":{"scope":"session"},"docs":{"scope":"session"}}},
                        "swarm":{"a":1}}}),
        json!({"tools":{"web":{"brave":{"enabled":false}},
                        "policy":{"entries":{"docs":{"scope":"parent"}}},
                        "swarm":{"b":2}}}),
    );
    assert_eq!(
        out,
        json!({"tools":{"web":{"brave":{"enabled":false,"max_results":9},"fetch":{"enabled":false}},
                        "policy":{"entries":{"exec":{"scope":"session"},"docs":{"scope":"parent"}}},
                        "swarm":{"b":2}}})
    );
}

#[test]
fn a_local_container_default_undefaults_the_global_entries() {
    let out = merged(
        json!({"container_configs":{"g":{"default":true,"create":["a"]},"h":{"create":["b"]}}}),
        json!({"container_configs":{"l":{"default":true,"create":["c"]}}}),
    );
    assert_eq!(
        out,
        json!({"container_configs":{"g":{"default":false,"create":["a"]},"h":{"create":["b"]},
                                     "l":{"default":true,"create":["c"]}}})
    );
    let out = merged(
        json!({"container_configs":{"g":{"default":true}}}),
        json!({"container_configs":{"g":{"exec":["x"]}}}),
    );
    assert_eq!(
        out,
        json!({"container_configs":{"g":{"exec":["x"]}}}),
        "entries replace whole"
    );
}

#[test]
fn workflow_merges_field_wise_with_templates_replaced_whole() {
    let out = merged(
        json!({"workflow":{"auto_continue":true,"templates":[{"id":"g"}]}}),
        json!({"workflow":{"templates":[{"id":"l"}]}}),
    );
    assert_eq!(
        out,
        json!({"workflow":{"auto_continue":true,"templates":[{"id":"l"}]}})
    );
}

#[test]
fn sections_absent_from_the_global_file_are_taken_from_the_overlay() {
    let out = merged(
        json!({}),
        json!({"agents":{"defaults":{"model":"l"}},"custom":1}),
    );
    assert_eq!(out, json!({"agents":{"defaults":{"model":"l"}},"custom":1}));
    let out = merged(
        json!({"agents":"junk"}),
        json!({"agents":{"defaults":{"model":"l"}}}),
    );
    assert_eq!(
        out,
        json!({"agents":{"defaults":{"model":"l"}}}),
        "a non-object base is replaced"
    );
}

#[test]
fn global_only_keys_are_named() {
    let doc = json!({"agents":{},"admission":null})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(global_only_key(&doc), Some("admission"));
    let doc = json!({"providers":{}}).as_object().unwrap().clone();
    assert_eq!(global_only_key(&doc), Some("providers"));
    let doc = json!({"agents":{}}).as_object().unwrap().clone();
    assert_eq!(global_only_key(&doc), None);
}

#[test]
fn dotted_paths_read_and_write_through_objects_only() {
    let mut doc = json!({"agents":{"defaults":{"model":"g"}},"list":[1]});
    assert_eq!(get_path(&doc, "agents.defaults.model"), Some(&json!("g")));
    assert_eq!(get_path(&doc, "agents.missing"), None);
    assert_eq!(get_path(&doc, ""), None);
    assert_eq!(get_path(&doc, "agents..model"), None);
    assert_eq!(get_path(&doc, "list.0"), None);

    set_path(
        &mut doc,
        "tools.policy.entries.exec",
        json!({"scope":"session"}),
    )
    .unwrap();
    assert_eq!(
        doc["tools"],
        json!({"policy":{"entries":{"exec":{"scope":"session"}}}})
    );
    assert_eq!(
        set_path(&mut doc, "list.0", json!(1)),
        Err("list".to_string())
    );
    assert_eq!(
        set_path(&mut doc, "agents.defaults.model.x", json!(1)),
        Err("agents.defaults.model".to_string())
    );
    assert_eq!(set_path(&mut doc, "", json!(1)), Err(String::new()));
    let mut scalar = json!("nope");
    assert_eq!(set_path(&mut scalar, "a", json!(1)), Err(String::new()));
}
