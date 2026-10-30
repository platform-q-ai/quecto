//! Completion protocol owned by the extension/admission success responders.
use serde_json::{Value, json};

/// Distinguish declaration continuations from initial work, never corrections.
pub(crate) fn continuation(body: &Value) -> bool {
    let messages = body["messages"].as_array().expect("provider messages");
    assert!(
        messages.iter().all(|message| message["content"].as_str()
            != Some(quecto::domain::agents::value_objects::turn_outcome::CORRECTION_NOTE)),
        "success fixture received forbidden correction: {body}"
    );
    let current: Vec<_> = messages
        .iter()
        .rev()
        .take_while(|message| matches!(message["role"].as_str(), Some("assistant" | "tool")))
        .collect();
    let declaration = current.iter().position(|message| {
        message["tool_calls"].as_array().is_some_and(|calls| {
            calls.iter().any(|call| {
                call["id"] == "integration-done"
                    && call["function"]["name"] == "declare_turn_outcome"
            })
        })
    });
    if let Some(declaration) = declaration {
        let receipt = current[..declaration]
            .iter()
            .find(|message| {
                message["role"] == "tool" && message["tool_call_id"] == "integration-done"
            })
            .expect("current declaration must have its matching receipt");
        let receipt: Value = serde_json::from_str(receipt["content"].as_str().unwrap())
            .expect("structured declaration receipt");
        assert_eq!(receipt["intent"], "done", "{receipt}");
        assert!(receipt["sessionKey"].is_string(), "{receipt}");
        for key in ["outcomeEpoch", "turnId"] {
            assert!(
                receipt[key]
                    .as_str()
                    .is_some_and(|s| s.chars().next().is_some()),
                "{receipt}"
            );
        }
        return true;
    }
    false
}

pub(crate) fn completion(body: &Value, text: &str) -> (Value, &'static str) {
    let offered = body["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "declare_turn_outcome")
    });
    match (offered, offered && continuation(body)) {
        (true, false) => (
            json!({"role":"assistant","content":null,"tool_calls":[{
                "index":0,"id":"integration-done","type":"function",
                "function":{"name":"declare_turn_outcome","arguments":json!({
                    "outcome":"done","reason":"The requested fixture work is complete."
                }).to_string()}
            }]}),
            "tool_calls",
        ),
        _ => (json!({"role":"assistant","content":text}), "stop"),
    }
}

/// Both wire formats carry the same completion and finish reason.
pub(crate) fn response(body: &Value, text: &str) -> (String, &'static str) {
    let (message, finish) = completion(body, text);
    let usage = json!({"prompt_tokens":1,"completion_tokens":1});
    if body["stream"] == true {
        let chunk = json!({"id":"fake","object":"chat.completion.chunk",
            "choices":[{"index":0,"delta":message,"finish_reason":finish}],"usage":usage});
        (
            format!("data: {chunk}\n\ndata: [DONE]\n\n"),
            "text/event-stream",
        )
    } else {
        (
            json!({"id":"fake","object":"chat.completion",
            "choices":[{"index":0,"message":message,"finish_reason":finish}],"usage":usage})
            .to_string(),
            "application/json",
        )
    }
}

#[cfg(test)]
#[path = "success_fixture_tests.rs"]
mod tests;
