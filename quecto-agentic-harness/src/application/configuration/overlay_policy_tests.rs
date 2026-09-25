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

#[test]
fn a_document_looks_like_a_config_only_with_a_known_section_shaped_as_one() {
    assert!(looks_like_config(&json!({"agents":{}})));
    assert!(looks_like_config(&json!({"custom":1,"workflow":{}})));
    assert!(
        looks_like_config(&json!({"admission":null})),
        "the #2023 shape: a quecto file that disabled admission"
    );
    assert!(!looks_like_config(&json!({"name":"app"})));
    assert!(!looks_like_config(&json!([])));
    // A known name with an unrelated value is another tool's file.
    assert!(!looks_like_config(&json!({"workflow":"build"})));
    assert!(!looks_like_config(&json!({"tools":["eslint"]})));
    assert!(!looks_like_config(&json!({"agents":null})));
    assert!(!looks_like_config(&json!({"providers":true})));
}

#[test]
fn remove_path_takes_the_value_out_and_reports_what_is_not_there() {
    let mut doc = json!({"agents":{"defaults":{"model":"m","effort":"high"}},"zeta":1});
    assert_eq!(
        remove_path(&mut doc, "agents.defaults.model"),
        Ok(Some(json!("m")))
    );
    assert_eq!(
        doc,
        json!({"agents":{"defaults":{"effort":"high"}},"zeta":1})
    );
    // Not set: a missing leaf, a missing intermediate object.
    assert_eq!(remove_path(&mut doc, "agents.defaults.model"), Ok(None));
    assert_eq!(remove_path(&mut doc, "tools.policy.entries"), Ok(None));
    assert_eq!(
        doc,
        json!({"agents":{"defaults":{"effort":"high"}},"zeta":1})
    );
    // Emptied parents are kept.
    assert_eq!(
        remove_path(&mut doc, "agents.defaults.effort"),
        Ok(Some(json!("high")))
    );
    assert_eq!(doc["agents"]["defaults"], json!({}));
    // A non-object on the way names the segment it stands at.
    assert_eq!(remove_path(&mut doc, "zeta.inner"), Err("zeta".to_string()));
    assert_eq!(remove_path(&mut doc, "",), Err(String::new()));
    let mut scalar = json!("nope");
    assert_eq!(remove_path(&mut scalar, "a"), Err(String::new()));
}

/// #2136: `tools.grep` merges field-wise per section, so an overlay turning
/// ranking off keeps the other global settings.
#[test]
fn tools_grep_merges_field_wise_per_section() {
    let global = serde_json::json!({"tools": {"grep": {
        "relevance": {"enabled": true, "max_candidates": 20},
        "log": {"enabled": true}
    }}});
    let overlay = serde_json::json!({"tools": {"grep": {"relevance": {"enabled": false}}}});
    let merged = merge_overlay(
        global.as_object().unwrap().clone(),
        overlay.as_object().unwrap().clone(),
    );
    assert_eq!(
        serde_json::Value::Object(merged)["tools"]["grep"],
        serde_json::json!({
            "relevance": {"enabled": false, "max_candidates": 20},
            "log": {"enabled": true}
        })
    );
}

/// #2136: an overlay may narrow grep's settings, never widen them: ranking
/// (which sends matching code to TypeSafe) off but not on, the search log
/// on but not off.
#[test]
fn an_overlay_may_narrow_grep_settings_but_not_widen_them() {
    let refused = |document: serde_json::Value| global_only_key(document.as_object().unwrap());
    assert_eq!(
        refused(json!({"tools": {"grep": {"relevance": {"enabled": true}}}})),
        Some("tools.grep.relevance")
    );
    assert_eq!(
        refused(json!({"tools": {"grep": {"relevance": {"enabled": false, "max_candidates": 5}}}})),
        Some("tools.grep.relevance"),
        "only switching it off is allowed"
    );
    assert_eq!(
        refused(json!({"tools": {"grep": {"relevance": null}}})),
        Some("tools.grep.relevance")
    );
    assert_eq!(
        refused(json!({"tools": {"grep": {"relevance": {"enabled": false}}}})),
        None
    );
    assert_eq!(
        refused(json!({"tools": {"grep": {"log": {"enabled": false}}}})),
        Some("tools.grep.log")
    );
    assert_eq!(
        refused(json!({"tools": {"grep": {"log": {"enabled": true}}}})),
        None
    );
    assert_eq!(
        refused(json!({"tools": {"grep": {"relevance": {}, "log": {}}}})),
        None,
        "no effect"
    );
    assert_eq!(refused(json!({"tools": {"web": {}}})), None);
    assert_eq!(
        global_only_path(&["providers", "openai"]),
        Some("providers")
    );
    assert_eq!(
        global_only_path(&["tools", "grep", "relevance", "enabled"]),
        None
    );
}
