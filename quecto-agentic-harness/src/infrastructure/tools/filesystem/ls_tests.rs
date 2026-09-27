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
async fn test_ls_lists_files_and_dirs() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("a.txt"), "").unwrap();
    std::fs::create_dir(tmp.path().join("subdir")).unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("a.txt"));
    assert!(result.content.contains("subdir/"));
}

#[tokio::test]
async fn test_ls_defaults_to_workspace() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("file.txt"), "").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("file.txt"));
}

#[tokio::test]
async fn test_ls_subdirectory() {
    let (ws, sb, tmp) = test_tools();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub/inner.txt"), "x").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"path": "sub"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("inner.txt"));
}

#[tokio::test]
async fn test_ls_allows_path_outside_workspace() {
    let (ws, sb, _tmp) = test_tools();
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("outside.txt"), "ok").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool
        .execute(&format!(r#"{{"path": "{}"}}"#, outside.path().display()))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("outside.txt"));
}

// --- Quecto compatibility ---

#[tokio::test]
async fn test_ls_empty_directory_message() {
    let (ws, sb, _tmp) = test_tools();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "(empty directory)");
}

#[tokio::test]
async fn test_ls_case_insensitive_sort() {
    let (ws, sb, tmp) = test_tools();
    std::fs::write(tmp.path().join("Makefile"), "").unwrap();
    std::fs::write(tmp.path().join("app.rs"), "").unwrap();
    std::fs::write(tmp.path().join("Zoo.rs"), "").unwrap();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    let lines: Vec<&str> = result.content.lines().collect();
    // Case-insensitive: app.rs < Makefile < Zoo.rs
    let idx_app = lines.iter().position(|&l| l == "app.rs").unwrap();
    let idx_make = lines.iter().position(|&l| l == "Makefile").unwrap();
    let idx_zoo = lines.iter().position(|&l| l == "Zoo.rs").unwrap();
    assert!(
        idx_app < idx_make && idx_make < idx_zoo,
        "expected app.rs < Makefile < Zoo.rs, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_ls_limit_parameter() {
    let (ws, sb, tmp) = test_tools();
    for i in 0..20 {
        std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 5}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("[Entries 1-5 of 20 shown"),
        "expected limit notice, got: {}",
        result.content
    );
    assert!(
        result.content.contains("offset=5"),
        "expected the next offset, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_ls_default_limit_is_500() {
    let (ws, sb, tmp) = test_tools();
    for i in 0..600 {
        std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("[Entries 1-500 of 600 shown"),
        "expected 500 limit notice, got: {}",
        &result.content[result.content.len().saturating_sub(200)..]
    );
}

#[tokio::test]
async fn test_ls_float_limit() {
    let (ws, sb, tmp) = test_tools();
    for i in 0..20 {
        std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 5.0}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("[Entries 1-5 of 20 shown"),
        "expected limit notice, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_ls_invalid_json_is_tool_error() {
    let (ws, sb, _tmp) = test_tools();
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid JSON arguments"));
}

/// Review 3: as grep's whole numbers, a limit that rounds below 1 is
/// refused, one that rounds to 1 or more is taken.
#[tokio::test]
async fn test_ls_float_limit_rounds_and_a_limit_below_one_is_refused() {
    let (ws, sb, tmp) = test_tools();
    for name in ["a", "b", "c"] {
        std::fs::write(tmp.path().join(name), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 0.6}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content.lines().next(), Some("a"));
    assert!(result.content.contains("[Entries 1-1 of 3 shown"));
    assert!(result.content.contains("offset=1"));
    for value in ["0", "0.4", "-0.4"] {
        let result = tool
            .execute(&format!(r#"{{"limit": {value}}}"#))
            .await
            .unwrap();
        assert!(result.is_error, "{value}");
        assert!(
            result.content.starts_with(&format!(
                "invalid 'limit' {value}: use a whole number of at least 1"
            )),
            "{}",
            result.content
        );
    }
}

#[test]
fn test_ls_description_includes_example() {
    let (ws, sb, _tmp) = test_tools();
    let tool = LsTool::new(ws, sb);
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "ls description should include Example, got: {}",
        def.description
    );
}

fn names(tmp: &TempDir, count: usize) {
    for i in 1..=count {
        std::fs::write(tmp.path().join(format!("f{i}")), "").unwrap();
    }
}

fn sorted_names(count: usize) -> Vec<String> {
    let mut all: Vec<String> = (1..=count).map(|i| format!("f{i}")).collect();
    all.sort_by_key(|name| (name.to_lowercase(), name.clone()));
    all
}

fn listed(content: &str) -> Vec<&str> {
    content
        .lines()
        .take_while(|line| !line.starts_with('['))
        .collect()
}

/// #2188: a truncated listing is the sorted prefix of the WHOLE directory,
/// not a sorted sample of whatever readdir returned first.
#[tokio::test]
async fn a_truncated_listing_is_the_first_entries_of_the_whole_directory() {
    let (ws, sb, tmp) = test_tools();
    names(&tmp, 1200);
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 20}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(listed(&result.content), sorted_names(1200)[..20]);
    assert!(
        result
            .content
            .ends_with("[Entries 1-20 of 1200 shown (sorted case-insensitively; limit 20 reached). Next: offset=20. Or raise limit (max 5000), or list a more specific path]"),
        "{}",
        result.content
    );
}

/// #2188: offset continues exactly where the previous page stopped.
#[tokio::test]
async fn offset_pages_through_the_sorted_directory() {
    let (ws, sb, tmp) = test_tools();
    names(&tmp, 45);
    let tool = LsTool::new(ws, sb);
    let all = sorted_names(45);
    let mut seen = Vec::new();
    for offset in [0, 20, 40] {
        let result = tool
            .execute(&format!(r#"{{"limit": 20, "offset": {offset}}}"#))
            .await
            .unwrap();
        assert!(!result.is_error);
        seen.extend(listed(&result.content).into_iter().map(str::to_owned));
    }
    assert_eq!(seen, all);
    let last = tool
        .execute(r#"{"limit": 20, "offset": 40}"#)
        .await
        .unwrap();
    assert!(
        !last.content.contains('['),
        "the last page has no note: {}",
        last.content
    );
    let middle = tool
        .execute(r#"{"limit": 20, "offset": 20}"#)
        .await
        .unwrap();
    assert!(
        middle.content.contains("[Entries 21-40 of 45 shown"),
        "{}",
        middle.content
    );
    assert!(
        middle.content.contains("Next: offset=40"),
        "{}",
        middle.content
    );
}

#[tokio::test]
async fn an_offset_past_the_end_says_how_many_entries_there_are() {
    let (ws, sb, tmp) = test_tools();
    names(&tmp, 3);
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"offset": 3}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(
        result.content,
        "(no entries at offset 3; the directory has 3 entries)"
    );
}

#[tokio::test]
async fn offset_is_rounded() {
    let (ws, sb, tmp) = test_tools();
    names(&tmp, 3);
    let tool = LsTool::new(ws, sb);
    let all = sorted_names(3);
    for (offset, first) in [("-0.4", 0), ("0.4", 0), ("1.6", 2), ("null", 0)] {
        let result = tool
            .execute(&format!(r#"{{"offset": {offset}}}"#))
            .await
            .unwrap();
        assert!(!result.is_error, "{offset}: {}", result.content);
        assert_eq!(listed(&result.content)[0], all[first], "offset {offset}");
    }
    let at_most = tool
        .execute(&format!(r#"{{"offset": {LS_MAX_OFFSET}}}"#))
        .await
        .unwrap();
    assert!(!at_most.is_error, "{}", at_most.content);
}

/// Review 2: an offset past the maximum or a negative number is refused,
/// never clamped to a page that was not asked for.
#[tokio::test]
async fn out_of_range_numbers_are_refused() {
    let (ws, sb, tmp) = test_tools();
    names(&tmp, 3);
    let tool = LsTool::new(ws, sb);
    let past = LS_MAX_OFFSET + 1;
    for (args, said) in [
        (
            format!(r#"{{"offset": {past}}}"#),
            format!("invalid 'offset' {past}: at most {LS_MAX_OFFSET}"),
        ),
        // The refusal echoes what was sent, not a saturated number.
        (
            r#"{"offset": 1e30}"#.to_string(),
            format!("invalid 'offset' 1e+30: at most {LS_MAX_OFFSET}"),
        ),
        (
            r#"{"offset": -4}"#.to_string(),
            "invalid 'offset' -4: use a whole number of at least 0".to_string(),
        ),
        (
            r#"{"limit": -1}"#.to_string(),
            "invalid 'limit' -1: use a whole number of at least 1".to_string(),
        ),
        (
            r#"{"limit": -0.6}"#.to_string(),
            "invalid 'limit' -0.6".to_string(),
        ),
    ] {
        let result = tool.execute(&args).await.unwrap();
        assert!(result.is_error, "{args}: {}", result.content);
        assert!(
            result.content.starts_with(&said),
            "{args}: {}",
            result.content
        );
    }
    let past = tool
        .execute(&format!(r#"{{"offset": {}}}"#, LS_MAX_OFFSET + 1))
        .await
        .unwrap();
    assert!(
        past.content.contains("use find with a pattern"),
        "{}",
        past.content
    );
    // A limit above the maximum is the maximum, as documented.
    let big = tool.execute(r#"{"limit": 1e9}"#).await.unwrap();
    assert!(!big.is_error, "{}", big.content);
}

/// The byte cap cuts at a whole entry and says where to continue.
#[tokio::test]
async fn the_byte_cap_keeps_whole_entries_and_names_the_next_offset() {
    let (ws, sb, tmp) = test_tools();
    for i in 0..400 {
        std::fs::write(tmp.path().join(format!("{i:03}{}", "x".repeat(200))), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    let result = tool.execute(r#"{"limit": 400}"#).await.unwrap();
    let shown = listed(&result.content);
    assert!(
        shown.iter().all(|line| line.len() == 203),
        "no partial entry"
    );
    let body: usize = shown.iter().map(|line| line.len() + 1).sum::<usize>() - 1;
    assert!(body <= LS_MAX_BYTES);
    assert!(body + 204 > LS_MAX_BYTES, "as many whole entries as fit");
    let note = format!(
        "[Entries 1-{0} of 400 shown (sorted case-insensitively; 50KB output cap reached). Next: offset={0}.",
        shown.len()
    );
    assert!(
        result.content.contains(&note),
        "{}",
        &result.content[result.content.len() - 300..]
    );
}

#[tokio::test]
async fn equal_folded_names_have_a_fixed_order() {
    let (ws, sb, tmp) = test_tools();
    for name in ["b", "B", "a", "A"] {
        std::fs::write(tmp.path().join(name), "").unwrap();
    }
    let tool = LsTool::new(ws, sb);
    for limit in [1, 2, 3, 4] {
        let result = tool
            .execute(&format!(r#"{{"limit": {limit}}}"#))
            .await
            .unwrap();
        assert_eq!(listed(&result.content), ["A", "a", "B", "b"][..limit]);
    }
}

#[test]
fn the_window_keeps_the_smallest_keys_in_any_arrival_order() {
    let mut window = SortedWindow::new(3);
    for name in ["e", "b", "D", "a", "c", "f/"] {
        window.offer(name.to_owned());
    }
    assert_eq!(window.total(), 6);
    assert_eq!(window.into_sorted(), ["a", "b", "c"]);
    let mut empty = SortedWindow::new(2);
    assert_eq!(empty.total(), 0);
    empty.offer("z".into());
    assert_eq!(empty.into_sorted(), ["z"]);
}

#[test]
fn the_description_and_schema_offer_offset() {
    let (ws, sb, _tmp) = test_tools();
    let def = LsTool::new(ws, sb).definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert_eq!(schema["properties"]["offset"]["type"], "number");
    assert!(def.description.contains("offset"), "{}", def.description);
}

/// Entries filling the byte cap exactly are all shown, with no note.
#[test]
fn entries_exactly_at_the_byte_cap_are_all_shown() {
    let mut window = SortedWindow::new(2);
    window.offer("a".repeat(LS_MAX_BYTES - 2));
    window.offer("b".into());
    let output = render_listing(window, 0, 2);
    assert_eq!(output.len(), LS_MAX_BYTES);
    assert!(
        output.ends_with("\nb"),
        "no note: {}",
        &output[output.len() - 20..]
    );
}

/// Review: a limit or offset that is not a number is refused with an
/// example, not silently replaced by the default.
#[tokio::test]
async fn a_non_numeric_limit_or_offset_is_refused() {
    let (ws, sb, tmp) = test_tools();
    names(&tmp, 3);
    let tool = LsTool::new(ws, sb);
    for (name, value) in [
        ("limit", r#""7""#),
        ("offset", r#""1""#),
        ("limit", "true"),
        ("offset", "[1]"),
        ("limit", "{}"),
    ] {
        let result = tool
            .execute(&format!(r#"{{"{name}": {value}}}"#))
            .await
            .unwrap();
        assert!(result.is_error, "{name}={value}: {}", result.content);
        assert!(
            result
                .content
                .starts_with(&format!("invalid '{name}' {value}: use a whole number")),
            "{}",
            result.content
        );
        assert!(
            result.content.contains(r#""offset": 100"#),
            "{}",
            result.content
        );
    }
    for args in [r#"{"limit": null, "offset": null}"#, r#"{}"#] {
        let result = tool.execute(args).await.unwrap();
        assert!(!result.is_error, "{args}");
        assert_eq!(listed(&result.content), sorted_names(3));
    }
}

#[test]
fn the_description_says_the_whole_directory_is_read() {
    let (ws, sb, _tmp) = test_tools();
    let def = LsTool::new(ws, sb).definition();
    assert!(
        def.description.contains("The whole directory is read"),
        "{}",
        def.description
    );
}
