//! Protocol-valid completion for success-only OpenAI fixture responders.
//! Explicitly unguarded children keep their original text-only response.
use serde_json::{Value, json};

pub(super) fn completion(body: &Value, text: &str) -> (Value, &'static str) {
    let offered = body["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "declare_turn_outcome")
    });
    let declared = body["messages"].as_array().is_some_and(|messages| {
        messages
            .iter()
            .rev()
            .take_while(|message| matches!(message["role"].as_str(), Some("assistant" | "tool")))
            .any(|message| {
                message["tool_calls"].as_array().is_some_and(|calls| {
                    calls
                        .iter()
                        .any(|call| call["function"]["name"] == "declare_turn_outcome")
                })
            })
    });
    if let (true, false) = (offered, declared) {
        return (
            json!({"role":"assistant","content":null,"tool_calls":[{
                "index":0,"id":"fixture-done","type":"function",
                "function":{"name":"declare_turn_outcome","arguments":json!({
                    "outcome":"done","reason":"The requested fixture work is complete."
                }).to_string()}
            }]}),
            "tool_calls",
        );
    }
    if let (true, true) = (offered, declared) {
        let receipt = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .take_while(|message| matches!(message["role"].as_str(), Some("assistant" | "tool")))
            .find(|message| message["role"] == "tool" && message["tool_call_id"] == "fixture-done")
            .expect("completion must follow the declaration's tool receipt");
        let receipt: Value = serde_json::from_str(receipt["content"].as_str().unwrap())
            .expect("declaration receipt is structured JSON");
        assert_eq!(receipt["intent"], "done", "{receipt}");
        assert!(receipt["sessionKey"].is_string(), "{receipt}");
        for key in ["outcomeEpoch", "turnId"] {
            assert!(
                receipt[key]
                    .as_str()
                    .is_some_and(|value| value.chars().next().is_some()),
                "{receipt}"
            );
        }
    }
    (json!({"role":"assistant","content":text}), "stop")
}

pub(super) fn response(body: &Value, mut response: Value) -> Value {
    if let Some(text) = response["choices"][0]["message"]["content"].as_str() {
        let (message, finish) = completion(body, text);
        response["choices"][0]["message"] = message;
        response["choices"][0]["finish_reason"] = json!(finish);
    }
    response
}

#[cucumber::then("success fixtures declare only offered completion after work and receipt")]
fn completion_contract(_world: &mut crate::QuectoWorld) {
    let legacy = json!({"messages":[{"role":"user","content":"work"}]});
    assert_eq!(
        completion(&legacy, "finished"),
        (json!({"role":"assistant","content":"finished"}), "stop")
    );
    let mut root = legacy;
    root["tools"] = json!([{"function":{"name":"declare_turn_outcome"}}]);
    let (declaration, finish) = completion(&root, "finished");
    assert_eq!(finish, "tool_calls");
    let arguments: Value = serde_json::from_str(
        declaration["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(arguments["outcome"], "done");
    root["messages"].as_array_mut().unwrap().extend([
        declaration,
        json!({"role":"tool","tool_call_id":"fixture-done","content":json!({
            "intent":"done","sessionKey":"root","outcomeEpoch":"epoch","turnId":"turn"
        }).to_string()}),
    ]);
    assert_eq!(
        completion(&root, "finished"),
        (json!({"role":"assistant","content":"finished"}), "stop")
    );
    root["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"user","content":"new work"}));
    assert_eq!(
        completion(&root, "next").1,
        "tool_calls",
        "a prior turn's receipt cannot complete a fresh instruction"
    );
}
