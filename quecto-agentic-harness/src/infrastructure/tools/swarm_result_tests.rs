use super::output_last;

/// The crate keeps JSON key order (serde_json's preserve_order): the swarm
/// result's layout depends on it (#2181 review).
#[test]
fn json_objects_keep_their_key_order() {
    let value = serde_json::json!({"zeta": 1, "alpha": 2});
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["zeta", "alpha"]);
}

/// Fields added after the output still leave it last.
#[test]
fn the_output_ends_the_result() {
    let mut result = serde_json::json!({"status": "completed", "stdout": "o", "stderr": "e"});
    result["notification_warnings"] = serde_json::json!(["w"]);
    output_last(&mut result);
    let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["status", "notification_warnings", "stdout", "stderr"]
    );
}
