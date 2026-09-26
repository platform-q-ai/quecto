use super::*;

fn blocks() -> Vec<ThinkingBlock> {
    vec![
        ThinkingBlock::Normal {
            thinking: "abc".into(),
            signature: String::new(),
        },
        ThinkingBlock::EncryptedReasoning {
            origin: "m".into(),
            leads_to: None,
            item: "{}".into(),
        },
        ThinkingBlock::Redacted { data: "x".into() },
    ]
}

/// #2162: encrypted reasoning takes no room in what a person sees.
#[test]
fn encrypted_reasoning_is_not_visible_thinking() {
    assert_eq!(visible_thinking_len(&blocks()), 4);
    assert_eq!(
        visible_thinking_page(&blocks(), 0, 4),
        vec![
            VisibleThinkingPageBlock::Text { text: "abc".into() },
            VisibleThinkingPageBlock::Redacted,
        ]
    );
    assert!(!blocks()[1].is_visible());
    assert!(blocks()[0].is_visible() && blocks()[2].is_visible());
}
