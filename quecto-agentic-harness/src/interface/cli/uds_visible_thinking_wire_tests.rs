use super::*;

/// #2162: the wire carries visible thinking only.
#[test]
fn the_wire_leaves_out_encrypted_reasoning() {
    let blocks = vec![
        ThinkingBlock::EncryptedReasoning {
            model: "m".into(),
            item: "SECRET".into(),
        },
        ThinkingBlock::Redacted { data: "x".into() },
    ];
    assert_eq!(
        visible_thinking_blocks_json(&blocks),
        serde_json::json!([{ "kind": "redacted" }])
    );
}
