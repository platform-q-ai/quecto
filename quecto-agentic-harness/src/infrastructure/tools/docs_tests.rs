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
    assert!(
        result
            .content
            .contains("Quecto parent-agent quick start and workflows playbook")
    );
    assert!(result.content.contains("subagents — "));
    assert!(result.content.contains("workflow — "));
    assert!(result.content.contains("extensions — "));
    assert!(result.content.contains("models — "));
    assert!(result.content.contains("rust-ast-graph — "));
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
async fn rust_ast_graph_manual_page_documents_agent_usage_examples() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"rust-ast-graph"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("rust_ast_graph"));
    assert!(result.content.contains("find_symbol"));
    assert!(result.content.contains("references"));
    assert!(result.content.contains("syntax-derived"));
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

/// #1319: top-level TOC still lists quick-start.
#[tokio::test]
async fn top_level_toc_includes_quick_start() {
    let tool = DocsTool::new();
    let result = tool.execute("{}").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("quick-start — "));
}

/// #1319: spawned children omit quick-start from the TOC.
#[tokio::test]
async fn spawned_toc_omits_quick_start() {
    let tool = DocsTool::for_child_content();
    let result = tool.execute("{}").await.unwrap();
    assert!(!result.is_error);
    assert!(
        !result.content.contains("quick-start"),
        "spawned TOC must omit quick-start; got:\n{}",
        result.content
    );
    // Other pages remain available.
    assert!(result.content.contains("subagents — "));
    assert!(result.content.contains("workflow — "));
    assert!(result.content.contains("extensions — "));
    assert!(result.content.contains("models — "));
}

/// #1319: direct retrieval of quick-start is rejected for spawned children.
#[tokio::test]
async fn spawned_rejects_direct_quick_start() {
    let tool = DocsTool::for_child_content();
    let result = tool.execute(r#"{"name":"quick-start"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("No embedded doc named"));
    assert!(
        !result.content.contains("Parent versus subagent"),
        "must not return quick-start body"
    );
}

/// #1319: aliases like docs/quick-start.md are also rejected when spawned.
#[tokio::test]
async fn spawned_rejects_quick_start_aliases() {
    let tool = DocsTool::for_child_content();
    for name in [
        "quick-start.md",
        "docs/quick-start.md",
        "docs/docs-tool-embeds/quick-start.md",
        "QUICK-START",
    ] {
        let args = format!(r#"{{"name":"{name}"}}"#);
        let result = tool.execute(&args).await.unwrap();
        assert!(
            result.is_error,
            "spawned must reject alias {name}; got ok content:\n{}",
            result.content
        );
        assert!(
            !result.content.contains("Parent versus subagent"),
            "alias {name} must not return quick-start body"
        );
    }
}

/// #1319: non-parent pages remain readable for spawned children.
#[tokio::test]
async fn spawned_can_read_other_manual_pages() {
    let tool = DocsTool::for_child_content();
    let result = tool.execute(r#"{"name":"workflow"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("# Workflow"));
}

/// #1319: top-level direct retrieval of quick-start is unchanged.
#[tokio::test]
async fn top_level_quick_start_still_available() {
    let tool = DocsTool::with_content_policy(DocsContentPolicy::Parent);
    let result = tool.execute(r#"{"name":"quick-start"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Parent versus subagent"));
}

#[test]
fn subagents_embed_teaches_container_environments() {
    let doc = lookup_doc("subagents").expect("subagents embed");
    for needle in [
        "Container spawning",
        "container: true",
        "container_config",
        "Available container configs",
        "sandbox",
        "\"mode\":\"existing\"",
        "environment_ref=C1",
        "get_containers",
        "kill_container",
        "absolute path",
        "parent's own effective config path",
    ] {
        assert!(doc.contains(needle), "subagents embed misses {needle}");
    }
}
