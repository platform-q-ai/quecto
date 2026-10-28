use super::*;

fn request(stream: bool, offered: bool) -> Value {
    let mut body = json!({"stream":stream,"messages":[{"role":"user","content":"same work"}]});
    if offered {
        body["tools"] = json!([{"function":{"name":"declare_turn_outcome"}}]);
    }
    body
}

fn append_receipt(body: &mut Value) {
    let (declaration, finish) = completion(body, "original prose");
    assert_eq!(finish, "tool_calls");
    let args: Value = serde_json::from_str(
        declaration["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(args["outcome"], "done");
    body["messages"].as_array_mut().unwrap().extend([
        declaration,
        json!({"role":"tool","tool_call_id":"integration-done","content":json!({
            "intent":"done","sessionKey":"","outcomeEpoch":"current-epoch","turnId":"current-turn"
        }).to_string()}),
    ]);
}

fn wire_message(body: &Value) -> (Value, String) {
    let (wire, content_type) = response(body, "original prose");
    let value: Value = if body["stream"] == true {
        assert_eq!(content_type, "text/event-stream");
        assert!(wire.ends_with("data: [DONE]\n\n"));
        serde_json::from_str(wire.lines().next().unwrap().strip_prefix("data: ").unwrap()).unwrap()
    } else {
        assert_eq!(content_type, "application/json");
        serde_json::from_str(&wire).unwrap()
    };
    let choice = &value["choices"][0];
    let message = if body["stream"] == true {
        &choice["delta"]
    } else {
        &choice["message"]
    };
    (
        message.clone(),
        choice["finish_reason"].as_str().unwrap().to_string(),
    )
}

#[test]
fn offered_absent_current_stale_and_repeated_prompt_in_both_wire_formats() {
    for stream in [false, true] {
        let legacy = request(stream, false);
        let (message, finish) = wire_message(&legacy);
        assert_eq!(message["content"], "original prose");
        assert_eq!(finish, "stop");
        let mut root = request(stream, true);
        assert_eq!(wire_message(&root).1, "tool_calls");
        append_receipt(&mut root);
        assert!(continuation(&root));
        let (message, finish) = wire_message(&root);
        assert_eq!(message["content"], "original prose");
        assert_eq!(finish, "stop");
        // Identical prompt text is a new episode; old receipt has no authority.
        root["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"user","content":"same work"}));
        assert!(!continuation(&root));
        assert_eq!(wire_message(&root).1, "tool_calls");
        append_receipt(&mut root);
        assert_eq!(wire_message(&root).1, "stop");
    }
}

#[test]
fn prose_requires_a_matching_current_structured_done_receipt() {
    for stream in [false, true] {
        for invalid in [
            json!({"intent":"asking","sessionKey":"","outcomeEpoch":"e","turnId":"t"}),
            json!({"intent":"done","outcomeEpoch":"e","turnId":"t"}),
            json!({"intent":"done","sessionKey":"","outcomeEpoch":"","turnId":"t"}),
            json!({"intent":"done","sessionKey":"","outcomeEpoch":"e","turnId":""}),
        ] {
            let mut body = request(stream, true);
            append_receipt(&mut body);
            body["messages"][2]["content"] = json!(invalid.to_string());
            assert!(std::panic::catch_unwind(|| response(&body, "original prose")).is_err());
        }
        let mut body = request(stream, true);
        append_receipt(&mut body);
        body["messages"][2]["tool_call_id"] = json!("stale-call");
        assert!(std::panic::catch_unwind(|| response(&body, "original prose")).is_err());
        body["messages"].as_array_mut().unwrap().pop();
        assert!(std::panic::catch_unwind(|| response(&body, "original prose")).is_err());
        let mut stale = request(stream, true);
        append_receipt(&mut stale);
        let declaration = stale["messages"][1].clone();
        stale["messages"].as_array_mut().unwrap().push(declaration);
        assert!(std::panic::catch_unwind(|| response(&stale, "original prose")).is_err());
        stale["tools"] = json!([]);
        assert_eq!(
            wire_message(&stale).1,
            "stop",
            "absent tools retain legacy prose"
        );
    }
}
