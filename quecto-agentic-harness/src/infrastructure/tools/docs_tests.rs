use super::*;

#[test]
fn lookup_doc_resolves_plain_md_and_prefixed_names() {
    assert!(lookup_doc("quecto").is_some());
    assert!(lookup_doc("quecto.md").is_some());
    assert!(lookup_doc("docs/quecto.md").is_some());
    assert!(lookup_doc("  SUBAGENTS  ").is_some());
    assert!(lookup_doc("nope").is_none());
}

#[tokio::test]
async fn execute_without_name_lists_available_docs() {
    let tool = DocsTool::new();
    let result = tool.execute("{}").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("quecto"));
    assert!(result.content.contains("subagents"));
    assert!(result.content.contains("contributor-cookbooks"));
}

#[tokio::test]
async fn execute_with_name_returns_doc_body() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"subagents"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("agent_cmd"));
}

#[tokio::test]
async fn execute_returns_contributor_cookbooks() {
    let tool = DocsTool::new();
    let result = tool
        .execute(r#"{"name":"contributor-cookbooks"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("## Add a built-in tool"));
    assert!(result.content.contains("## Local check command index"));
}

#[tokio::test]
async fn execute_accepts_md_suffix_and_docs_prefix() {
    let tool = DocsTool::new();
    let result = tool
        .execute(r#"{"name":"docs/subagents.md"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("agent_cmd"));
}

#[tokio::test]
async fn execute_unknown_doc_is_error_and_lists_available() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"nonexistent"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("quecto"));
}

#[tokio::test]
async fn execute_with_invalid_json_lists_docs() {
    let tool = DocsTool::new();
    let result = tool.execute("not json").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Available quecto capability docs"));
}
