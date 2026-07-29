use super::*;

#[test]
fn lookup_doc_resolves_plain_md_and_prefixed_names() {
    assert!(lookup_doc("quick-start").is_some());
    assert!(lookup_doc("subagents").is_some());
    assert!(lookup_doc("subagents.md").is_some());
    assert!(lookup_doc("docs/subagents.md").is_some());
    assert!(lookup_doc("docs/docs-tool-embeds/workflow.md").is_some());
    assert!(lookup_doc("  MODELS  ").is_some());
    assert!(lookup_doc("quecto").is_none());
    assert!(lookup_doc("readme").is_none());
    assert!(lookup_doc("uds-protocol").is_none());
    assert!(lookup_doc("sessions").is_none());
    assert!(lookup_doc("contributor-cookbooks").is_none());
    assert!(lookup_doc("nope").is_none());
}

#[test]
fn doc_title_reads_first_h1() {
    assert_eq!(doc_title("# Hello world\n\nbody"), Some("Hello world"));
    assert_eq!(doc_title("no title\n## Section"), None);
}

#[tokio::test]
async fn execute_without_name_lists_toc_with_titles() {
    let tool = DocsTool::new();
    let result = tool.execute("{}").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("operating manual"));
    assert!(result.content.contains("Table of contents:"));
    assert!(result.content.contains("quick-start — "));
    assert!(result.content.contains("Quecto parent-agent quick start"));
    assert!(result.content.contains("subagents — "));
    assert!(result.content.contains("workflow — "));
    assert!(result.content.contains("extensions — "));
    assert!(result.content.contains("models — "));
    assert!(!result.content.contains("contributor-cookbooks"));
    assert!(!result.content.contains("uds-protocol"));
}

#[tokio::test]
async fn execute_with_name_returns_doc_body() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"quick-start"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Parent versus subagent"));
    assert!(result.content.contains("get_messages"));
}

#[tokio::test]
async fn execute_returns_concise_subagents_deep_dive() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"subagents"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("get_messages"));
    assert!(result.content.contains("read_only"));
    // Must stay a deep-dive, not the old full manual.
    assert!(result.content.len() < 8_000);
}

#[tokio::test]
async fn execute_accepts_md_suffix_and_docs_prefix() {
    let tool = DocsTool::new();
    let result = tool
        .execute(r#"{"name":"docs/quick-start.md"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Parent versus subagent"));
}

#[tokio::test]
async fn execute_unknown_doc_is_error_and_lists_toc() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"nonexistent"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("quick-start"));
    assert!(result.content.contains("Table of contents:"));
}

#[tokio::test]
async fn execute_with_invalid_json_lists_docs() {
    let tool = DocsTool::new();
    let result = tool.execute("not json").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("operating manual"));
}
