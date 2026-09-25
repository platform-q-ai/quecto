use crate::QuectoWorld;
use cucumber::{given, then, when};
use quecto::application::search::ports::{
    PortFuture, Relevance, RelevanceCandidate, RelevanceJudge,
};
use quecto::application::tools::ports::Tool;
use quecto::infrastructure::search::search_log::JsonlSearchLog;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::grep::GrepTool;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_grep_workspace(world: &mut QuectoWorld) -> PathBuf {
    if world.grep_workspace.is_none() {
        let tmp = TempDir::new().expect("failed to create grep temp dir");
        let path = tmp.path().to_path_buf();
        world._grep_temp_dir = Some(tmp);
        world.grep_workspace = Some(path);
    }
    world.grep_workspace.clone().unwrap()
}

fn make_grep_tool(world: &mut QuectoWorld) -> GrepTool {
    let ws = ensure_grep_workspace(world);
    let ws_arc = Arc::new(ws.clone());
    // validate_path is not a filesystem jail.
    let sandbox = Arc::new(Sandbox::new(Some(ws.clone())));
    let mut tool = GrepTool::new(ws_arc, sandbox);
    if let Some(word) = world.grep_favours.clone() {
        tool = tool.with_relevance(Arc::new(FavouringJudge(word)), 30);
    }
    if world.grep_judge_down {
        tool = tool.with_relevance(Arc::new(DownJudge), 30);
    }
    if let Some(dir) = world.grep_log_dir.clone() {
        tool = tool.with_search_log(Arc::new(JsonlSearchLog::new(&dir, "bdd-session")));
    }
    tool
}

/// A stand-in for TypeSafe's judge (#2136): hits whose text holds its word
/// are relevant, others are not.
struct FavouringJudge(String);

impl RelevanceJudge for FavouringJudge {
    fn judge<'a>(
        &'a self,
        _query: &'a str,
        candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance> {
        let scores = candidates
            .iter()
            .map(|c| {
                Some(if c.matched.contains(&self.0) {
                    0.9
                } else {
                    0.1
                })
            })
            .collect();
        Box::pin(async move { Relevance::Scored(scores) })
    }
}

/// A stand-in for TypeSafe when it cannot answer.
struct DownJudge;

impl RelevanceJudge for DownJudge {
    fn judge<'a>(
        &'a self,
        _query: &'a str,
        _candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance> {
        Box::pin(async { Relevance::Unavailable("TypeSafe answered HTTP 529".to_string()) })
    }
}

/// The search log's records, oldest first.
fn search_log_records(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let dir = world
        .grep_log_dir
        .as_ref()
        .expect("the grep search log was set up")
        .join("search-log");
    let mut records = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the search log directory exists") {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        records.extend(text.lines().map(|line| serde_json::from_str(line).unwrap()));
    }
    records
}

fn run_tool(tool: GrepTool, args: serde_json::Value) -> quecto::domain::tool::ToolResult {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { tool.execute(&args.to_string()).await })
        .unwrap_or_else(|e| quecto::domain::tool::ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        })
}

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("a grep tool workspace")]
fn given_grep_workspace(world: &mut QuectoWorld) {
    ensure_grep_workspace(world);
}

#[given(regex = r#"^a grep workspace file "([^"]+)" with content:$"#)]
fn given_grep_file_with_docstring(
    world: &mut QuectoWorld,
    step: &cucumber::gherkin::Step,
    filename: String,
) {
    let ws = ensure_grep_workspace(world);
    let content = step
        .docstring
        .as_deref()
        .unwrap_or("")
        .trim_start_matches('\n');
    let path = ws.join(&filename);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("failed to create the grep file's directory");
    }
    std::fs::write(path, content).expect("failed to write grep file");
}

#[given(regex = r#"^a grep workspace file "([^"]+)" with 200 lines containing "([^"]+)"$"#)]
fn given_grep_file_many_lines(world: &mut QuectoWorld, filename: String, word: String) {
    let ws = ensure_grep_workspace(world);
    let content: String = (1..=200)
        .map(|i| format!("line {}: {}\n", i, word))
        .collect();
    std::fs::write(ws.join(&filename), content).expect("failed to write many-line grep file");
}

#[given(
    regex = r#"^a grep workspace file "([^"]+)" with (\d+) lines of (\d+) chars containing "([^"]+)"$"#
)]
fn given_grep_file_long_lines(
    world: &mut QuectoWorld,
    filename: String,
    line_count: usize,
    chars: usize,
    word: String,
) {
    let ws = ensure_grep_workspace(world);
    let padding = "x".repeat(chars.saturating_sub(word.len()));
    let content: String = (1..=line_count)
        .map(|_| format!("{}{}\n", word, padding))
        .collect();
    std::fs::write(ws.join(&filename), content).expect("failed to write long-line grep file");
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when(regex = r#"^I grep for pattern "([^"]+)"$"#)]
fn when_grep_pattern(world: &mut QuectoWorld, pattern: String) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with ignoreCase (true|false)$"#)]
fn when_grep_ignore_case(world: &mut QuectoWorld, pattern: String, flag: String) {
    let tool = make_grep_tool(world);
    let ignore_case = flag == "true";
    let args = serde_json::json!({ "pattern": pattern, "ignoreCase": ignore_case });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with literal (true|false)$"#)]
fn when_grep_literal(world: &mut QuectoWorld, pattern: String, flag: String) {
    let tool = make_grep_tool(world);
    let literal = flag == "true";
    let args = serde_json::json!({ "pattern": pattern, "literal": literal });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with glob "([^"]+)"$"#)]
fn when_grep_glob(world: &mut QuectoWorld, pattern: String, glob: String) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "glob": glob });
    world.grep_result = Some(run_tool(tool, args));
}

/// Any arguments, as the model sends them (#2136).
#[when(regex = r#"^I grep with arguments:$"#)]
fn when_grep_with_arguments(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    let tool = make_grep_tool(world);
    let text = step
        .docstring
        .as_deref()
        .expect("the grep arguments follow as a docstring");
    let args: serde_json::Value = serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("grep arguments are JSON: {e}: {text}"));
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with limit (\d+)$"#)]
fn when_grep_limit(world: &mut QuectoWorld, pattern: String, limit: usize) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "limit": limit });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with context (\d+)$"#)]
fn when_grep_context(world: &mut QuectoWorld, pattern: String, context: u64) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "context": context });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep with missing rg binary for pattern "([^"]+)"$"#)]
fn when_grep_missing_binary(world: &mut QuectoWorld, pattern: String) {
    let ws = ensure_grep_workspace(world);
    let ws_arc = Arc::new(ws.clone());
    let sandbox = Arc::new(Sandbox::new(Some(ws.clone())));
    let tool = GrepTool::with_rg_binary(
        ws_arc,
        sandbox,
        "/nonexistent/path/to/rg_binary_xyz".to_string(),
    );
    let args = serde_json::json!({ "pattern": pattern });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" in path "([^"]+)"$"#)]
fn when_grep_outside_workspace(world: &mut QuectoWorld, pattern: String, path: String) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "path": path });
    world.grep_result = Some(run_tool(tool, args));
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then(regex = r#"^the grep result should contain "([^"]+)"$"#)]
fn then_grep_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    assert!(
        result.content.contains(&expected),
        "grep result should contain {:?}, got:\n{}",
        expected,
        result.content
    );
}

#[then(regex = r#"^the grep result should not contain "([^"]+)"$"#)]
fn then_grep_result_not_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    assert!(
        !result.content.contains(&expected),
        "grep result should NOT contain {:?}, got:\n{}",
        expected,
        result.content
    );
}

#[then(regex = r#"^the grep result should list "([^"]+)" before "([^"]+)"$"#)]
fn then_grep_lists_before(world: &mut QuectoWorld, first: String, second: String) {
    let result = world.grep_result.as_ref().expect("grep result");
    let text = &result.content;
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("grep result lacks {needle:?}: {text}"))
    };
    assert!(
        at(&first) < at(&second),
        "{first:?} should come before {second:?}: {text}"
    );
}

#[given(regex = r#"^grep ranks matches with a stand-in judge that favours "([^"]+)"$"#)]
fn given_grep_judge(world: &mut QuectoWorld, word: String) {
    world.grep_favours = Some(word);
}

#[given("grep ranks matches with a judge that cannot answer")]
fn given_grep_judge_down(world: &mut QuectoWorld) {
    world.grep_judge_down = true;
}

#[given("grep records its searches in a local search log")]
fn given_grep_search_log(world: &mut QuectoWorld) {
    let base = TempDir::new().expect("a base dir for the search log");
    world.grep_log_dir = Some(base.path().to_path_buf());
    world._grep_log_temp_dir = Some(base);
}

#[then(regex = r#"^the search log should hold (\d+) searches?$"#)]
fn then_search_log_holds(world: &mut QuectoWorld, count: usize) {
    let records = search_log_records(world);
    assert_eq!(records.len(), count, "{records:?}");
}

#[then(regex = r#"^search (\d+) in the search log should be "([^"]+)" output that found (\d+)$"#)]
fn then_search_logged(world: &mut QuectoWorld, index: usize, output: String, found: u64) {
    let records = search_log_records(world);
    let record = &records[index - 1];
    assert_eq!(record["output"], output.as_str(), "{record}");
    assert_eq!(record["found"], found, "{record}");
    assert_eq!(record["session"], "bdd-session", "{record}");
}

#[then(regex = r#"^search (\d+) in the search log should record ranking "([^"]+)"$"#)]
fn then_search_ranking_logged(world: &mut QuectoWorld, index: usize, status: String) {
    let records = search_log_records(world);
    assert_eq!(
        records[index - 1]["ranking"]["status"],
        status.as_str(),
        "{}",
        records[index - 1]
    );
}

#[then("the grep result should not be an error")]
fn then_grep_not_error(world: &mut QuectoWorld) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    if result.content.contains("rg not found on PATH")
        || result.content.starts_with("rg not available")
    {
        return;
    }
    assert!(
        !result.is_error,
        "grep result should not be an error, got:\n{}",
        result.content
    );
}

#[then("the grep result should be an error")]
fn then_grep_is_error(world: &mut QuectoWorld) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    assert!(
        result.is_error,
        "grep result should be an error, got:\n{}",
        result.content
    );
}
