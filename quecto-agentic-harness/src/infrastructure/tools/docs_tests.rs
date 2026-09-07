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

#[test]
fn default_constructs_the_parent_docs_tool() {
    let tool: DocsTool = Default::default();
    assert_eq!(tool.definition().name.as_ref(), "docs");
    assert!(!tool.definition().description.is_empty());
}

#[tokio::test]
async fn execute_without_name_lists_toc_with_titles() {
    let tool = DocsTool::new();
    let result = tool.execute("{}").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("operating manual"));
    assert!(result.content.contains("Table of contents:"));
    assert!(result.content.contains("quick-start — "));
    assert!(result.content.contains("Quecto agent quick start"));
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
    assert!(result.content.contains("Spawn and recover results"));
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
    assert!(result.content.contains("Spawn and recover results"));
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

/// Given a child runtime, the shared quick start is listed and readable via every alias.
#[tokio::test]
async fn spawned_can_read_shared_quick_start_and_aliases() {
    let child = DocsTool::for_child_content();
    let parent = DocsTool::new();
    assert_eq!(
        child.definition().description,
        parent.definition().description
    );
    assert!(child.definition().description.contains("quick-start"));
    let toc = child.execute("{}").await.unwrap();
    assert!(!toc.is_error);
    assert!(
        toc.content
            .contains("quick-start — Quecto agent quick start")
    );
    assert_eq!(toc.content, parent.execute("{}").await.unwrap().content);
    for name in [
        "quick-start",
        "quick-start.md",
        "docs/quick-start.md",
        "docs/docs-tool-embeds/quick-start.md",
        "QUICK-START",
    ] {
        let args = format!(r#"{{"name":"{name}"}}"#);
        let result = child.execute(&args).await.unwrap();
        assert!(!result.is_error, "alias {name}: {}", result.content);
        assert_eq!(result.content, parent.execute(&args).await.unwrap().content);
        assert!(result.content.contains("Spawn and recover results"));
        assert!(!result.content.contains("Delegate to a subagent when"));
        assert!(!result.content.contains("prefer a child with"));
        assert!(!result.content.contains("Run an `adversarial-review` child"));
        assert!(!result.content.contains("Parent-agent identity"));
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
    assert!(result.content.contains("Spawn and recover results"));
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
