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
}

#[tokio::test]
async fn execute_with_name_returns_doc_body() {
    let tool = DocsTool::new();
    let result = tool.execute(r#"{"name":"subagents"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("agent_cmd"));
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

fn write_skill(base: &std::path::Path, name: &str, description: &str, body: &str) {
    let dir = base.join("workspace").join("skills").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
    )
    .unwrap();
}

#[tokio::test]
async fn execute_lists_legacy_skills_as_knowledge_docs() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "review",
        "Review code",
        "Legacy review guidance",
    );
    let tool = DocsTool::with_workspace(tmp.path().join("workspace"));

    let result = tool.execute("{}").await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("skills/review"));
    assert!(result.content.contains("Review code"));
}

#[tokio::test]
async fn execute_fetches_legacy_skill_body_on_demand() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "review",
        "Review code",
        "Legacy review guidance",
    );
    let tool = DocsTool::with_workspace(tmp.path().join("workspace"));

    let result = tool.execute(r#"{"name":"skills/review"}"#).await.unwrap();

    assert!(!result.is_error);
    assert_eq!(result.content, "Legacy review guidance");
}
