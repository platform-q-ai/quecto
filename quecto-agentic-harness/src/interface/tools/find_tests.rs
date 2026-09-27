use super::*;
use crate::application::agent_turn::use_cases::find::{
    FindEntryKind, FindOutput, FindPaths, FindPathsRequest,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<FindPathsRequest>>,
    response: Mutex<Option<Result<FindOutput, FindError>>>,
}
impl FindPaths for Fake {
    fn find(
        &self,
        request: FindPathsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FindOutput, FindError>> + Send + '_>> {
        self.calls.lock().unwrap().push(request);
        Box::pin(async {
            self.response
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(FindOutput::default()))
        })
    }
}
fn fixture() -> (FindTool, Arc<Fake>) {
    let fake = Arc::new(Fake::default());
    (FindTool::new(FindUseCase::new(fake.clone())), fake)
}

#[tokio::test]
async fn invalid_required_syntax_never_invokes_effect() {
    let (tool, fake) = fixture();
    for input in [
        "{",
        "{}",
        "null",
        "[]",
        "42",
        "\"text\"",
        "{\"pattern\":null}",
        "{\"pattern\":2}",
        "{\"pattern\":[]}",
    ] {
        let result = tool.execute(input).await.unwrap();
        assert!(result.is_error, "{input}");
        assert!(result.content.contains("Example:"));
    }
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn optional_fallbacks_empty_pattern_and_unknown_fields_are_accepted() {
    let (tool, fake) = fixture();
    for input in [
        r#"{"pattern":""}"#,
        r#"{"pattern":"","path":null,"limit":null}"#,
        r#"{"pattern":"","path":42,"limit":null,"extra":true}"#,
    ] {
        assert!(!tool.execute(input).await.unwrap().is_error);
    }
    assert_eq!(
        *fake.calls.lock().unwrap(),
        vec![
            FindPathsRequest {
                pattern: "".into(),
                path: ".".into(),
                limit: 1000,
                kind: None,
            };
            3
        ]
    );
    tool.execute(r#"{"pattern":"*.rs","path":"src","limit":5.5}"#)
        .await
        .unwrap();
    assert_eq!(
        fake.calls.lock().unwrap()[3],
        FindPathsRequest {
            pattern: "*.rs".into(),
            path: "src".into(),
            limit: 6,
            kind: None,
        }
    );
}

#[tokio::test]
async fn errors_keep_their_delivery_channels() {
    let (tool, fake) = fixture();
    for (error, kind) in [
        (FindError::Security("security".into()), 0),
        (FindError::Spawn("install fd-find".into()), 1),
        (FindError::Search("find error: bad glob".into()), 2),
        (FindError::Io("read failed".into()), 2),
    ] {
        *fake.response.lock().unwrap() = Some(Err(error));
        match (kind, tool.execute(r#"{"pattern":"*"}"#).await) {
            (0, Err(DomainError::Security(message))) => assert_eq!(message, "security"),
            (1, Err(DomainError::Tool(message))) => assert_eq!(message, "install fd-find"),
            (2, Ok(result)) => assert!(result.is_error),
            other => panic!("wrong channel: {other:?}"),
        }
    }
}

fn rendered(entries: Vec<String>, incomplete: bool, limited: bool) -> String {
    render(FindResult {
        output: FindOutput {
            entries,
            incomplete,
            result_limit_reached: limited,
            diagnostic: None,
            skipped_vcs_dir: None,
        },
        limit: 3,
        kind: None,
    })
}

#[test]
fn renders_order_whitespace_directories_unicode_and_limit_notice() {
    assert!(rendered(vec![], false, false).starts_with("No files found matching pattern"));
    assert_eq!(
        rendered(vec!["z/".into(), " ".into(), "文�.rs".into()], false, false),
        "z/\n \n文�.rs"
    );
    assert!(
        rendered(vec!["a".into()], false, true).contains(
            "Results limit reached: the search stops at its limit, so this listing is an arbitrary subset of the matches, not the first. Use limit=6"
        )
    );
}

#[test]
fn exact_byte_cap_is_allowed_and_over_cap_never_fabricates_paths() {
    let cap = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;
    assert_eq!(
        rendered(vec!["x".repeat(cap)], false, false),
        "x".repeat(cap)
    );
    assert_eq!(
        rendered(vec!["x".repeat(cap + 1)], false, false),
        "[50KB limit reached]"
    );
    assert_eq!(
        rendered(vec!["ok".into(), "文".repeat(cap)], false, false),
        "ok\n[50KB limit reached]"
    );
    assert_eq!(
        rendered(vec!["x".repeat(cap - 2), "y".into()], false, false).len(),
        cap
    );
}

#[test]
fn incompleteness_survives_empty_output_and_other_hints() {
    for entries in [vec![], vec!["path".into()], vec!["x".repeat(60_000)]] {
        let output = rendered(entries, true, true);
        assert!(output.contains("incomplete"));
        assert!(!output.contains("No files found"));
    }
    let output = render(FindResult {
        output: FindOutput {
            incomplete: true,
            diagnostic: Some("producer stopped".into()),
            ..Default::default()
        },
        limit: 1,
        kind: None,
    });
    assert!(output.contains("producer stopped"));
}

#[test]
fn stable_tool_definition() {
    let (tool, _) = fixture();
    let definition = tool.definition();
    assert_eq!(definition.name, "find");
    let schema: serde_json::Value = serde_json::from_str(&definition.parameters_schema).unwrap();
    assert_eq!(schema["required"], serde_json::json!(["pattern"]));
    assert_eq!(schema["properties"]["limit"]["type"], "number");
}

#[tokio::test]
async fn successful_invocation_renders_metadata_without_changing_error_flag() {
    let (tool, fake) = fixture();
    *fake.response.lock().unwrap() = Some(Ok(FindOutput {
        entries: vec!["a.rs".into()],
        incomplete: true,
        result_limit_reached: true,
        diagnostic: Some("partial search".into()),
        skipped_vcs_dir: None,
    }));
    let result = tool.execute(r#"{"pattern":"*","limit":1}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.starts_with("a.rs\n"));
    assert!(result.content.contains("incomplete"));
    assert!(result.content.contains("partial search"));
    assert!(result.image_blocks.is_empty());
    assert!(result.delivery_metadata.is_none());
}

/// #2176 review: a listing cut by the byte cap still says it is drawn from
/// an arbitrary subset when the limit was also reached.
#[test]
fn the_byte_cap_keeps_the_limit_notice() {
    let long = "x".repeat(200);
    let entries: Vec<String> = (0..400).map(|i| format!("{long}{i}")).collect();
    let out = rendered(entries, false, true);
    assert!(out.contains("[50KB limit reached]"), "{out}");
    assert!(out.contains("an arbitrary subset of the matches"), "{out}");
}

/// #2200: type is read against an allowlist; anything else is refused
/// before any search, never silently widened to both kinds.
#[tokio::test]
async fn the_type_argument_is_an_allowlist() {
    let (tool, fake) = fixture();
    for (value, kind) in [
        (r#""f""#, Some(FindEntryKind::File)),
        (r#""file""#, Some(FindEntryKind::File)),
        (r#""d""#, Some(FindEntryKind::Directory)),
        (r#""directory""#, Some(FindEntryKind::Directory)),
        ("null", None),
    ] {
        let result = tool
            .execute(&format!(r#"{{"pattern":"*","type":{value}}}"#))
            .await
            .unwrap();
        assert!(!result.is_error, "{value}: {}", result.content);
        assert_eq!(
            fake.calls.lock().unwrap().pop().unwrap().kind,
            kind,
            "{value}"
        );
    }
    for value in [r#""x""#, r#""D""#, r#""""#, "1", "true", r#"["d"]"#, "{}"] {
        let result = tool
            .execute(&format!(r#"{{"pattern":"*","type":{value}}}"#))
            .await
            .unwrap();
        assert!(result.is_error, "{value}");
        assert!(
            result.content.contains(r#""f""#) && result.content.contains(r#""d""#),
            "{value}: {}",
            result.content
        );
    }
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[test]
fn no_matches_hint_at_the_type_filter_and_skipped_vcs_internals() {
    let empty = |kind, skipped: Option<&str>| {
        render(FindResult {
            output: FindOutput {
                skipped_vcs_dir: skipped.map(str::to_owned),
                ..Default::default()
            },
            limit: 3,
            kind,
        })
    };
    let plain = empty(None, None);
    assert!(
        plain.starts_with("No files found matching pattern"),
        "{plain}"
    );
    assert!(plain.contains(r#"type "d""#), "{plain}");
    assert!(
        !plain.contains("VCS"),
        "no skipped directory, no hint: {plain}"
    );
    assert!(plain.ends_with(')'), "{plain}");
    // Only a VCS directory that was really skipped is suggested, by the
    // path that reaches it (a worktree's real git directory, #2199 review).
    let skipped = empty(None, Some("main/.git/worktrees/wt"));
    assert!(
        skipped.ends_with(
            "; VCS metadata lives at main/.git/worktrees/wt, which is not searched unless passed as path)"
        ),
        "{skipped}"
    );
    // A directory kind may come from type "d" or from a trailing '/'.
    let directories = empty(Some(FindEntryKind::Directory), None);
    assert!(
        directories.starts_with("No directories found matching pattern"),
        "{directories}"
    );
    assert!(
        directories.contains(r#"type "d" or a pattern ending in '/'"#),
        "{directories}"
    );
    let files = empty(Some(FindEntryKind::File), Some(".git"));
    assert!(
        files.starts_with("No files found matching pattern"),
        "{files}"
    );
    assert!(files.contains(r#"type "f""#), "{files}");
    assert!(files.contains("VCS metadata lives at .git,"), "{files}");
}

#[test]
fn the_definition_offers_type_and_says_where_paths_are_relative_to() {
    let (tool, _) = fixture();
    let definition = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&definition.parameters_schema).unwrap();
    assert_eq!(
        schema["properties"]["type"]["enum"],
        serde_json::json!(["f", "d"])
    );
    for said in [
        "relative to the workspace",
        ".git",
        "type",
        "ends in '/'",
        "symlink counts as what it points at",
        "a symlink never does",
    ] {
        assert!(
            definition.description.contains(said),
            "{said}: {}",
            definition.description
        );
    }
    assert!(definition.description.len() <= 1024, "a short description");
}

/// Review 2: limit is a number, as for ls, read and grep; anything else
/// is refused before any search.
#[tokio::test]
async fn a_limit_that_is_not_a_number_is_refused() {
    let (tool, fake) = fixture();
    // As grep's whole numbers: a value that rounds below 1 is refused.
    for value in [r#""10""#, "true", "[5]", "{}", "-3", "0", "0.4", "-0.4"] {
        let result = tool
            .execute(&format!(r#"{{"pattern":"*","limit":{value}}}"#))
            .await
            .unwrap();
        assert!(result.is_error, "{value}");
        assert!(
            result
                .content
                .starts_with(&format!("invalid 'limit' {value}:")),
            "{}",
            result.content
        );
        assert!(
            result.content.contains(r#""limit": 100"#),
            "{}",
            result.content
        );
    }
    assert!(fake.calls.lock().unwrap().is_empty());
    for (value, normalized) in [("null", 1000), ("2.6", 3), ("0.6", 1), ("1e9", 100_000)] {
        let result = tool
            .execute(&format!(r#"{{"pattern":"*","limit":{value}}}"#))
            .await
            .unwrap();
        assert!(!result.is_error, "{value}: {}", result.content);
        assert_eq!(
            fake.calls.lock().unwrap().pop().unwrap().limit,
            normalized,
            "{value}"
        );
    }
}

/// Review 3: with nothing kept, a bigger limit is no advice.
#[test]
fn no_limit_advice_when_nothing_is_shown() {
    let shown = rendered(vec!["a".into()], true, true);
    assert!(shown.contains("Use limit=6"), "{shown}");
    let empty = rendered(vec![], true, true);
    assert!(!empty.contains("Use limit="), "{empty}");
    assert!(empty.contains("Search incomplete"), "{empty}");
}
