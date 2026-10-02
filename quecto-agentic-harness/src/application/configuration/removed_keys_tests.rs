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
