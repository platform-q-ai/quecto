use super::*;

#[test]
fn lookup_doc_resolves_plain_md_and_prefixed_names() {
    assert!(lookup_doc("quick-start").is_none());
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
    assert!(!result.content.contains("quick-start — "));
    assert!(result.content.contains("Workflow"));
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
    let result = tool.execute(r#"{"name":"workflow"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Workflow"));
    assert!(result.content.contains("workflow"));
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
        .execute(r#"{"name":"docs/workflow.md"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Workflow"));
}

#[tokio::test]
async fn execute_unknown_doc_is_error_and_lists_toc() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"nonexistent"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("workflow"));
    assert!(result.content.contains("Table of contents:"));
}

#[tokio::test]
async fn execute_with_invalid_json_lists_docs() {
    let tool = DocsTool::new();
    let result = tool.execute("not json").await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("operating manual"));
}

/// Removed quick-start is neither advertised nor retrievable by either role.
#[tokio::test]
async fn quick_start_is_removed_for_all_agents() {
    for tool in [DocsTool::new(), DocsTool::for_child_content()] {
        assert!(!tool.definition().description.contains("quick-start"));
        assert!(!tool.definition().parameters_schema.contains("quick-start"));
        let toc = tool.execute("{}").await.unwrap();
        assert!(!toc.content.contains("quick-start"));
        for name in [
            "quick-start",
            "quick-start.md",
            "docs/quick-start.md",
            "docs/docs-tool-embeds/quick-start.md",
            "QUICK-START",
        ] {
            assert!(lookup_doc(name).is_none());
            let result = tool
                .execute(&format!(r#"{{"name":"{name}"}}"#))
                .await
                .unwrap();
            assert!(result.is_error);
            assert!(result.content.contains("No embedded doc named"));
        }
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

#[tokio::test]
async fn embedded_swarm_manual_teaches_types_limits_and_terminal_reporting() {
    let result = DocsTool::for_child_content()
        .execute(r#"{"name":"swarm"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    for required in [
        "list[str]",
        "RLIMIT_NPROC",
        "bash",
        "accepted",
        "op=summary",
        "workspace-relative",
    ] {
        assert!(result.content.contains(required), "manual lacks {required}");
    }
}

/// #1707: admission help is discoverable and embedded for every agent role.
#[tokio::test]
async fn admission_broker_manual_is_discoverable_and_actionable() {
    for tool in [DocsTool::new(), DocsTool::for_child_content()] {
        let toc = tool.execute("{}").await.unwrap();
        assert!(toc.content.contains("admission-broker — Admission broker"));
        for name in ["admission-broker", "docs/admission-broker.md"] {
            let result = tool
                .execute(&format!(r#"{{"name":"{name}"}}"#))
                .await
                .unwrap();
            assert!(!result.is_error, "{}", result.content);
            assert_eq!(Some(result.content.as_str()), lookup_doc(name));
            for required in [
                "disabled by default",
                "max_scopes",
                "1024",
                "terminal_capacity",
                "4096",
                "openai-api",
                "openai-oauth",
                "queue_timeout_ms",
                "attempt_timeout_ms",
                "quecto admission-broker run",
                "quecto admission-broker status",
                "quecto admission-broker reset",
                "journal_healthy",
                "uncertain",
                "get_state",
                "timer",
                "parallel",
                "get_messages",
                "min_interval_ms",
                "\"tool_uses\"",
                "\"recipient_name\": \"functions.spawn\"",
                "left-panel",
                "monotonic",
                "rebases",
                "#1708",
            ] {
                assert!(result.content.contains(required), "manual lacks {required}");
            }
        }
    }
}
