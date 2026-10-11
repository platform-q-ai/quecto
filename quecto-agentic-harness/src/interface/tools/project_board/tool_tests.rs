use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::ThreadId;

use serde_json::json;

use super::*;
use crate::interface::tools::project_board::answer::ErrorCode;
use crate::interface::tools::project_board::boards::{memory_board, never_built};
use crate::interface::tools::project_board::request::OPS;
use crate::interface::tools::project_board::{
    BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

/// Bytes the description may take; it only shrinks, and stays inside the
/// 8,192-byte budget every tool description keeps.
const DESCRIPTION_BUDGET_BYTES: usize = 2_132;

fn tool(build: ProjectBoardHandlesBuilder) -> BoardTool {
    BoardTool::new(build, PathBuf::from("/srv/repo"))
}

fn call(build: ProjectBoardHandlesBuilder, arguments: &str) -> ToolResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(tool(build).execute(arguments)).unwrap()
}

fn answered(result: &ToolResult) -> Value {
    serde_json::from_str(&result.content).unwrap()
}

#[test]
fn the_description_is_the_approved_text_byte_for_byte() {
    let fixture = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/project_board/board_tool_description.txt"),
    )
    .unwrap();
    let definition = tool(never_built).definition();
    assert_eq!(definition.name, "board");
    assert_eq!(definition.description.as_bytes(), fixture.as_slice());
    assert!(
        definition.description.len() <= DESCRIPTION_BUDGET_BYTES
            && DESCRIPTION_BUDGET_BYTES <= 8_192,
        "the board description is {} bytes",
        definition.description.len()
    );
}

/// Words the description's op lists use beside field names.
const PROSE: &[&str] = &["any", "of", "at", "most", "one", "filter"];

#[test]
fn every_op_and_field_the_description_names_is_the_tools() {
    let text = BOARD_TOOL_DESCRIPTION;
    let statuses: Vec<&str> = TaskStatus::ALL.iter().map(|s| s.as_str()).collect();
    let mut named = Vec::new();
    for (at, _) in text.match_indices('(') {
        let op_start = text[..at]
            .rfind(|c: char| !(c.is_ascii_lowercase() || c == '_'))
            .map_or(0, |i| i + 1);
        let op = &text[op_start..at];
        let close = at + text[at..].find(')').unwrap();
        let args = &text[at + 1..close];
        if op.is_empty() {
            continue;
        }
        let fields = fields_of(op).unwrap_or_else(|| panic!("{op}( is no op"));
        assert!(TOOL_OPS.contains(&op), "{op} is not the tool's");
        let words: Vec<&str> = args
            .split(|c: char| !(c.is_ascii_lowercase() || c == '_'))
            .filter(|word| !word.is_empty())
            .collect();
        for word in &words {
            let known = fields.iter().any(|field| field.name == *word)
                || statuses.contains(word)
                || PROSE.contains(word);
            assert!(known, "{op}({args}): {word} is no field of {op}");
        }
        for field in fields {
            assert!(words.contains(&field.name), "{op} takes {}", field.name);
        }
        named.push(op);
    }
    for op in TOOL_OPS {
        let word = |at: usize| {
            let before = text[..at].chars().next_back();
            let after = text[at + op.len()..].chars().next();
            let edge =
                |c: Option<char>| !matches!(c, Some(c) if c.is_ascii_lowercase() || c == '_');
            edge(before) && edge(after)
        };
        assert!(
            text.match_indices(op).any(|(at, _)| word(at)),
            "the description never names {op}"
        );
    }
    assert!(named.len() >= 15, "{named:?}");
    assert!(
        text.contains(&format!("Statuses: {}.", statuses.join(", "))),
        "the statuses line lists TaskStatus::ALL"
    );
    for code in [
        ErrorCode::Stale,
        ErrorCode::Contended,
        ErrorCode::ClockAhead,
        ErrorCode::TaskAhead,
    ] {
        assert!(text.contains(&format!("\"{}\"", code.as_str())), "{code:?}");
    }
}

#[test]
fn the_tool_takes_every_shared_op_but_bootstrap() {
    let shared: Vec<&str> = OPS
        .iter()
        .map(|(op, _)| *op)
        .filter(|op| *op != "bootstrap")
        .collect();
    let mut tools = TOOL_OPS.to_vec();
    let mut shared_sorted = shared.clone();
    tools.sort_unstable();
    shared_sorted.sort_unstable();
    assert_eq!(tools, shared_sorted);
    let schema: Value =
        serde_json::from_str(&tool(never_built).definition().parameters_schema).unwrap();
    assert_eq!(schema["properties"]["op"]["enum"], json!(TOOL_OPS));
    assert_eq!(schema["required"], json!(["op"]));
    for op in TOOL_OPS {
        for field in fields_of(op).unwrap() {
            assert!(
                schema["properties"][field.name]["type"].is_string()
                    || schema["properties"][field.name]["type"].is_array(),
                "{op}.{} is untyped",
                field.name
            );
        }
    }
    assert_eq!(
        schema["properties"]["parent"]["type"],
        json!(["string", "null"])
    );
    assert_eq!(schema["properties"]["pr"]["minimum"], json!(1));
}

#[test]
fn a_call_is_answered_exactly_as_the_cli_answers_it() {
    for request in [
        json!({"op": "list"}),
        json!({"op": "list", "status": "ready"}),
        json!({"op": "show", "task": "a"}),
        json!({"op": "list_ready"}),
        json!({"op": "list_takeable"}),
        json!({"op": "claim", "task": "a"}),
        json!({"op": "move", "task": "a", "to": "claimed"}),
        json!({"op": "release", "task": "a"}),
        json!({"op": "archive", "task": "a"}),
        json!({"op": "block", "task": "a"}),
        json!({"op": "renew", "task": "nope"}),
        json!({"op": "create", "task": "b", "title": "B", "plan": [{"title": "S"}]}),
        json!({"op": "show", "task": "a", "extra": 1}),
    ] {
        let location = BoardLocation {
            repo: PathBuf::from("/srv/repo"),
            remote: DEFAULT_REMOTE.into(),
        };
        let cli = serve(Some(memory_board), location, &request);
        let result = call(memory_board, &request.to_string());
        assert_eq!(answered(&result), cli, "{request}");
        assert_eq!(result.is_error, cli["ok"] != json!(true), "{request}");
        if result.is_error {
            // The shape the description states: {"code","field","message"}.
            for key in ["code", "field", "message"] {
                assert!(cli["error"].get(key).is_some(), "{key}: {cli}");
            }
        }
    }
}

#[test]
fn claim_release_block_and_archive_are_moves_by_name() {
    let claimed = answered(&call(memory_board, r#"{"op":"claim","task":"a"}"#));
    assert_eq!(
        (&claimed["ok"], &claimed["op"], &claimed["task"]["status"]),
        (&json!(true), &json!("claim"), &json!("claimed"))
    );
    assert!(claimed["task"]["claim"]["expires"].is_string(), "{claimed}");
    let archived = answered(&call(memory_board, r#"{"op":"archive","task":"a"}"#));
    assert_eq!(archived["task"]["status"], json!("archived"));
    let blocked = answered(&call(memory_board, r#"{"op":"block","task":"a"}"#));
    assert_eq!(
        (&blocked["error"]["code"], &blocked["error"]["rule"]["code"]),
        (&json!("rule"), &json!("not_allowed"))
    );
}

#[test]
fn a_call_naming_no_op_of_the_tools_builds_nothing() {
    for arguments in [
        r#"{"op":"bootstrap"}"#,
        r#"{"op":"ready","task":"a"}"#,
        r#"{"op":7}"#,
        r#"{"task":"a"}"#,
        r#"[]"#,
    ] {
        let result = call(never_built, arguments);
        let answer = answered(&result);
        assert!(result.is_error, "{arguments}");
        assert_eq!(answer["error"]["code"], json!("invalid_request"));
        assert_eq!(answer["error"]["field"], json!("op"), "{arguments}");
        let problem = answer["error"]["problem"].as_str().unwrap();
        assert!(
            problem.contains("clear_set_aside") && !problem.contains("bootstrap"),
            "{problem}"
        );
    }
    for arguments in ["not json", r#"{"op":"show","op":"list"}"#] {
        let answer = answered(&call(never_built, arguments));
        assert_eq!(answer["error"]["field"], json!("arguments"), "{arguments}");
    }
    let answer = answered(&call(never_built, r#"{"op":"show","task":"Not A Slug"}"#));
    assert_eq!(
        (&answer["error"]["field"], &answer["op"]),
        (&json!("task"), &json!("show"))
    );
}

static BUILT_ON: Mutex<Option<ThreadId>> = Mutex::new(None);

fn recording(environment: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    *BUILT_ON.lock().unwrap() = Some(std::thread::current().id());
    assert_eq!(environment.repo, PathBuf::from("/srv/repo"));
    assert_eq!(environment.remote, DEFAULT_REMOTE);
    memory_board(environment)
}

#[test]
fn the_board_is_built_and_run_off_the_async_runtime() {
    let answer = answered(&call(recording, r#"{"op":"list"}"#));
    assert_eq!(answer["ok"], json!(true), "{answer}");
    let built_on = BUILT_ON.lock().unwrap().expect("the board was built");
    assert_ne!(built_on, std::thread::current().id());
}
