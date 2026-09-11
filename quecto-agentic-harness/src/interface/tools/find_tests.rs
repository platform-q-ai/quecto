use super::*;
use crate::application::agent_turn::use_cases::find::{FindOutput, FindPaths, FindPathsRequest};
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
        r#"{"pattern":"","path":42,"limit":"7","extra":true}"#,
    ] {
        assert!(!tool.execute(input).await.unwrap().is_error);
    }
    assert_eq!(
        *fake.calls.lock().unwrap(),
        vec![
            FindPathsRequest {
                pattern: "".into(),
                path: ".".into(),
                limit: 1000
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
            limit: 6
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
        },
        limit: 3,
    })
}

#[test]
fn renders_order_whitespace_directories_unicode_and_limit_heuristic() {
    assert_eq!(
        rendered(vec![], false, false),
        "No files found matching pattern"
    );
    assert_eq!(
        rendered(vec!["z/".into(), " ".into(), "文�.rs".into()], false, false),
        "z/\n \n文�.rs"
    );
    assert!(
        rendered(vec!["a".into()], false, true).contains("3 results limit reached. Use limit=6")
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
    }));
    let result = tool.execute(r#"{"pattern":"*","limit":1}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.starts_with("a.rs\n"));
    assert!(result.content.contains("incomplete"));
    assert!(result.content.contains("partial search"));
    assert!(result.image_blocks.is_empty());
    assert!(result.delivery_metadata.is_none());
}
