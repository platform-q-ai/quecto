use super::super::CodexProvider;

#[test]
fn parse_sse_separates_indexed_reasoning_summary_updates() {
    let sse = r#"data: {"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"delta":"Inspecting loop light activation logic"}
data: {"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":1,"delta":"Planning incremental loop light enhancements"}
data: {"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":2,"delta":"Proposing implementing loop IN/OUT lights"}
data: {"type":"response.completed","response":{"status":"completed"}}
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    match &resp.thinking_blocks[0] {
        crate::domain::message::ThinkingBlock::Normal { thinking, .. } => assert_eq!(
            thinking,
            "Inspecting loop light activation logic\n\nPlanning incremental loop light enhancements\n\nProposing implementing loop IN/OUT lights"
        ),
        other => panic!("unexpected thinking block: {other:?}"),
    }
}

#[test]
fn parse_sse_does_not_split_reasoning_on_non_summary_indexes() {
    let sse = r#"data: {"type":"response.reasoning_summary_text.delta","output_index":0,"content_index":0,"delta_index":0,"delta":"Reviewing Open"}
data: {"type":"response.reasoning_summary_text.delta","output_index":0,"content_index":1,"delta_index":1,"delta":"AI streaming parser"}
data: {"type":"response.completed","response":{"status":"completed"}}
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    match &resp.thinking_blocks[0] {
        crate::domain::message::ThinkingBlock::Normal { thinking, .. } => {
            assert_eq!(thinking, "Reviewing OpenAI streaming parser");
        }
        other => panic!("unexpected thinking block: {other:?}"),
    }
}
