use super::*;
use crate::infrastructure::security::sandbox::Sandbox;
use tempfile::TempDir;

fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf())));
    (workspace, sandbox, tmp)
}

#[tokio::test]
async fn test_read_repeated_unchanged_text_returns_marker() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("same.txt"), "alpha\nbeta\n").unwrap();

    let first = tool.execute(r#"{"path":"same.txt"}"#).await.unwrap();
    assert!(!first.is_error);
    assert!(first.content.contains("alpha"));

    let second = tool.execute(r#"{"path":"same.txt"}"#).await.unwrap();
    assert!(!second.is_error);
    assert!(
        second.content.contains("unchanged since read"),
        "{}",
        second.content
    );
    assert!(second.content.contains("2 lines"), "{}", second.content);
    assert!(!second.content.contains("alpha"), "{}", second.content);
}

#[tokio::test]
async fn test_read_force_bypasses_unchanged_marker() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("force.txt"), "fresh\ncontent\n").unwrap();

    tool.execute(r#"{"path":"force.txt"}"#).await.unwrap();
    let forced = tool
        .execute(r#"{"path":"force.txt","force":true}"#)
        .await
        .unwrap();
    assert!(!forced.is_error);
    assert!(forced.content.contains("fresh"));
    assert!(!forced.content.contains("unchanged since read"));

    let repeated = tool.execute(r#"{"path":"force.txt"}"#).await.unwrap();
    assert!(repeated.content.contains("unchanged since read"));
}

#[tokio::test]
async fn test_read_newline_only_text_change_updates_cache() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let path = tmp.path().join("newline.txt");
    std::fs::write(&path, "same\n").unwrap();
    tool.execute(r#"{"path":"newline.txt"}"#).await.unwrap();
    std::fs::write(&path, "same").unwrap();

    let changed = tool.execute(r#"{"path":"newline.txt"}"#).await.unwrap();
    assert!(!changed.is_error);
    assert_eq!(changed.content, "same");
}

#[tokio::test]
async fn test_read_modified_text_updates_cache() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let path = tmp.path().join("changed.txt");
    std::fs::write(&path, "old\n").unwrap();
    tool.execute(r#"{"path":"changed.txt"}"#).await.unwrap();
    std::fs::write(&path, "new\n").unwrap();

    let changed = tool.execute(r#"{"path":"changed.txt"}"#).await.unwrap();
    assert!(!changed.is_error);
    assert!(changed.content.contains("new"));
    assert!(!changed.content.contains("unchanged since read"));
    let repeated = tool.execute(r#"{"path":"changed.txt"}"#).await.unwrap();
    assert!(repeated.content.contains("unchanged since read"));
}

#[tokio::test]
async fn test_read_default_offset_and_offset_one_share_cache_scope() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("offset-one.txt"), "same\n").unwrap();
    tool.execute(r#"{"path":"offset-one.txt"}"#).await.unwrap();

    let second = tool
        .execute(r#"{"path":"offset-one.txt","offset":1}"#)
        .await
        .unwrap();
    assert!(
        second.content.contains("unchanged since read"),
        "{}",
        second.content
    );
}

#[tokio::test]
async fn test_read_different_ranges_do_not_cross_hit() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("ranges.txt"), "one\ntwo\nthree\nfour\n").unwrap();

    tool.execute(r#"{"path":"ranges.txt"}"#).await.unwrap();
    let ranged = tool
        .execute(r#"{"path":"ranges.txt","offset":2,"limit":2}"#)
        .await
        .unwrap();
    assert!(ranged.content.contains("two"));
    assert!(!ranged.content.contains("unchanged since read"));

    let same_ranged = tool
        .execute(r#"{"path":"ranges.txt","offset":2.0,"limit":2.0}"#)
        .await
        .unwrap();
    assert!(same_ranged.content.contains("unchanged since read"));
    let full_again = tool.execute(r#"{"path":"ranges.txt"}"#).await.unwrap();
    assert!(full_again.content.contains("unchanged since read"));
    let different_range = tool
        .execute(r#"{"path":"ranges.txt","offset":3,"limit":1}"#)
        .await
        .unwrap();
    assert!(different_range.content.contains("three"));
    assert!(!different_range.content.contains("unchanged since read"));
}

#[tokio::test]
async fn test_repeated_image_read_is_not_short_circuited() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let png_bytes: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
    std::fs::write(tmp.path().join("img.png"), png_bytes).unwrap();

    let first = tool.execute(r#"{"path":"img.png"}"#).await.unwrap();
    let second = tool.execute(r#"{"path":"img.png"}"#).await.unwrap();
    assert_eq!(first.image_blocks.len(), 1);
    assert_eq!(second.image_blocks.len(), 1);
    assert!(second.content.contains("Read image file"));
    assert!(!second.content.contains("unchanged since read"));
}

#[test]
fn test_read_definition_mentions_force() {
    let (ws, sb, _tmp) = test_tools();
    let def = ReadTool::new(ws, sb).definition();
    assert!(
        def.description.contains("force:true"),
        "{}",
        def.description
    );
    assert!(
        def.parameters_schema.contains("force"),
        "{}",
        def.parameters_schema
    );
}

#[tokio::test]
async fn test_read_limit_only_scope_short_circuits_on_repeat() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("limit-cache.txt"), "one\ntwo\nthree\n").unwrap();

    let first = tool
        .execute(r#"{"path":"limit-cache.txt","limit":2}"#)
        .await
        .unwrap();
    assert!(first.content.contains("one"));
    assert!(!first.content.contains("unchanged since read"));
    let second = tool
        .execute(r#"{"path":"limit-cache.txt","limit":2}"#)
        .await
        .unwrap();
    assert!(second.content.contains("unchanged since read"));
    assert!(!second.content.contains("one"));
}

#[tokio::test]
async fn test_read_default_limit_and_explicit_default_limit_do_not_share_cache_scope() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let content: String = (1..=100).map(|i| format!("line{i}\n")).collect();
    std::fs::write(tmp.path().join("default-limit.txt"), content).unwrap();

    tool.execute(r#"{"path":"default-limit.txt"}"#)
        .await
        .unwrap();
    let second = tool
        .execute(r#"{"path":"default-limit.txt","offset":1,"limit":2000}"#)
        .await
        .unwrap();
    assert!(!second.content.contains("unchanged since read"));
    assert!(second.content.contains("line1"));
}

#[tokio::test]
async fn test_read_default_scope_detects_changes_beyond_displayed_window() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let path = tmp.path().join("beyond-window.txt");
    let original: String = (1..=2101).map(|i| format!("line{i}\n")).collect();
    std::fs::write(&path, original).unwrap();
    tool.execute(r#"{"path":"beyond-window.txt"}"#)
        .await
        .unwrap();
    let modified: String = (1..=2100)
        .map(|i| format!("line{i}\n"))
        .chain(std::iter::once("changed-after-window\n".to_string()))
        .collect();
    std::fs::write(&path, modified).unwrap();

    let second = tool
        .execute(r#"{"path":"beyond-window.txt"}"#)
        .await
        .unwrap();
    assert!(!second.content.contains("unchanged since read"));
    assert!(second.content.contains("Use offset="));
}

#[tokio::test]
async fn test_read_default_truncated_scope_short_circuits_on_repeat() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    let content: String = (1..=2100).map(|i| format!("line{i}\n")).collect();
    std::fs::write(tmp.path().join("truncated-cache.txt"), content).unwrap();

    let first = tool
        .execute(r#"{"path":"truncated-cache.txt"}"#)
        .await
        .unwrap();
    assert!(first.content.contains("Use offset="));
    let second = tool
        .execute(r#"{"path":"truncated-cache.txt"}"#)
        .await
        .unwrap();
    assert!(second.content.contains("unchanged since read"));
    assert!(!second.content.contains("line1"));
}

#[tokio::test]
async fn test_read_force_false_null_and_non_boolean_do_not_bypass_marker() {
    let (ws, sb, tmp) = test_tools();
    let tool = ReadTool::new(ws, sb);
    std::fs::write(tmp.path().join("force-defaults.txt"), "body\n").unwrap();

    tool.execute(r#"{"path":"force-defaults.txt"}"#)
        .await
        .unwrap();
    for args in [
        r#"{"path":"force-defaults.txt","force":false}"#,
        r#"{"path":"force-defaults.txt","force":null}"#,
        r#"{"path":"force-defaults.txt","force":"true"}"#,
    ] {
        let result = tool.execute(args).await.unwrap();
        assert!(
            result.content.contains("unchanged since read"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("body"), "{}", result.content);
    }
}
