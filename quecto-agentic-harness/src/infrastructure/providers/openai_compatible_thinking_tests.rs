use super::openai::OpenAiProvider;
use crate::domain::message::ThinkingBlock;

#[test]
fn parse_response_extracts_exact_string_reasoning_content() {
    let body = serde_json::json!({
        "choices": [{"message": {
            "content": "answer",
            "reasoning_content": "visible reasoning",
            "signature": "PRIVATE_SIGNATURE"
        }}]
    });

    let response = OpenAiProvider::parse_response(&body).unwrap();
    assert_eq!(response.content.as_deref(), Some("answer"));
    assert_eq!(response.thinking_blocks.len(), 1);
    match &response.thinking_blocks[0] {
        ThinkingBlock::Normal {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "visible reasoning");
            assert!(signature.is_empty());
        }
        other => panic!("unexpected thinking block: {other:?}"),
    }
}

#[test]
fn parse_response_preserves_all_supported_reasoning_fields_in_order() {
    let body = serde_json::json!({
        "choices": [{"message": {
            "content": "answer",
            "reasoning_content": "first",
            "reasoning": "second",
            "thinking": "third"
        }}]
    });

    let response = OpenAiProvider::parse_response(&body).unwrap();
    let visible: Vec<_> = response
        .thinking_blocks
        .iter()
        .map(|block| match block {
            ThinkingBlock::Normal {
                thinking,
                signature,
            } => {
                assert!(signature.is_empty());
                thinking.as_str()
            }
            other => panic!("unexpected thinking block: {other:?}"),
        })
        .collect();
    assert_eq!(visible, vec!["first", "second", "third"]);
}

#[test]
fn parse_response_fails_closed_for_unsupported_reasoning_shapes() {
    for message in [
        serde_json::json!({"content": "answer", "reasoning": {"text": "private"}}),
        serde_json::json!({"content": "answer", "metadata": {"reasoning_content": "private"}}),
        serde_json::json!({"content": "answer", "reasoning_content": ["private"]}),
    ] {
        let body = serde_json::json!({"choices": [{"message": message}]});
        let response = OpenAiProvider::parse_response(&body).unwrap();
        assert!(response.thinking_blocks.is_empty());
    }
}
