use super::*;
use crate::interface::tools::project_board::ProjectBoardHandlesBuilder;
use crate::interface::tools::project_board::boards::{
    NO_COMMITTER, memory_board, never_built, unbuildable,
};
use serde_json::json;

fn args(line: &[&str]) -> Vec<String> {
    line.iter().map(|arg| (*arg).to_string()).collect()
}

fn built(line: &[&str], stdin: &str) -> Result<Value, RequestError> {
    let line = read_line(&args(line))?;
    let stdin = stdin.to_string();
    request(&line, || bounded(stdin.as_bytes()))
}

#[test]
fn the_command_line_builds_the_request_the_tool_would_send() {
    assert_eq!(
        built(&["link-pr", "--task", "a", "--pr", "7", "--json"], ""),
        Ok(json!({"op": "link_pr", "task": "a", "pr": 7}))
    );
    assert_eq!(
        built(&["create", "--input", r#"{"task": "a", "title": "A"}"#], ""),
        Ok(json!({"op": "create", "task": "a", "title": "A"}))
    );
    assert_eq!(
        built(&["--input", "-"], r#"{"op": "list_ready"}"#),
        Ok(json!({"op": "list_ready"}))
    );
    let line = read_line(&args(&["list", "--repo", "/srv/x", "--remote", "board"])).unwrap();
    assert_eq!(
        (line.repo, line.remote),
        (Some(PathBuf::from("/srv/x")), "board".into())
    );
    for (line, stdin, field) in [
        (&["renew", "--task"][..], "", "task"),
        (&["renew", "extra"][..], "", "extra"),
        (
            &["create", "--input", "{}", "--title", "A"][..],
            "",
            "input",
        ),
        (&["create", "--input", "[]"][..], "", "input"),
        (&["create", "--input", "not json"][..], "", "input"),
        (
            &["create", "--input", r#"{"task": "a", "task": "b"}"#][..],
            "",
            "input",
        ),
        (
            &["renew", "--input", r#"{"op": "move", "task": "a"}"#][..],
            "",
            "op",
        ),
        (&["--input", "-"][..], "\u{0}\u{ff}", "input"),
        (&["--json"][..], "", "op"),
    ] {
        assert_eq!(built(line, stdin).unwrap_err().field, field, "{line:?}");
    }
}

#[test]
fn input_from_stdin_is_bounded_and_its_failures_are_the_requests() {
    let big = "x".repeat(usize::try_from(MAX_REQUEST_BYTES).unwrap() + 1);
    let refused = built(&["--input", "-"], &big).unwrap_err();
    assert_eq!(refused.field, "input");
    assert!(refused.problem.contains("more than"), "{}", refused.problem);
    let line = read_line(&args(&["--input", "-"])).unwrap();
    let broken = request(&line, || Err(std::io::Error::other("the pipe broke"))).unwrap_err();
    assert_eq!(
        (
            broken.field.as_str(),
            broken.problem.contains("the pipe broke")
        ),
        ("input", true)
    );
}

fn cli(builder: Option<ProjectBoardHandlesBuilder>, line: &[&str]) -> (i32, Value, String) {
    let ctx = CliContext {
        project_board: builder,
        ..CliContext::default()
    };
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = cmd_board(&ctx, &args(line), &mut stdout, &mut stderr);
    let answer = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (code, answer, stderr)
}

#[test]
fn a_command_prints_one_json_answer_and_exits_by_it() {
    let (code, answer, _) = cli(
        Some(memory_board),
        &["move", "--task", "a", "--to", "claimed", "--json"],
    );
    assert_eq!(
        (code, &answer["ok"], &answer["task"]["status"]),
        (0, &json!(true), &json!("claimed"))
    );
    let (code, answer, _) = cli(Some(memory_board), &["renew", "--task", "nope"]);
    assert_eq!((code, &answer["error"]["code"]), (1, &json!("not_found")));
}

#[test]
fn a_request_is_read_before_anything_is_built() {
    let (code, answer, _) = cli(Some(never_built), &["renew", "--holder", "x"]);
    assert_eq!(
        (code, &answer["error"]["code"], &answer["op"]),
        (1, &json!("invalid_request"), &json!("renew"))
    );
    let (code, answer, _) = cli(Some(never_built), &["ready"]);
    assert_eq!((code, &answer["error"]["field"]), (1, &json!("op")));
}

#[test]
fn a_write_needs_a_committer_and_a_read_does_not() {
    let (code, answer, _) = cli(Some(memory_board), &["list", "--remote", NO_COMMITTER]);
    assert_eq!((code, &answer["ok"]), (0, &json!(true)));
    let (code, answer, _) = cli(
        Some(memory_board),
        &["renew", "--task", "a", "--remote", NO_COMMITTER],
    );
    assert_eq!(code, 1);
    assert_eq!(answer["error"]["code"], json!("no_committer"));
    assert_eq!(
        answer["error"]["reason"],
        json!("user.name is not configured")
    );
}

#[test]
fn a_board_that_cannot_be_built_or_is_not_composed_answers_why() {
    let (code, answer, _) = cli(Some(unbuildable), &["list_ready"]);
    assert_eq!(code, 1);
    assert_eq!(answer["error"]["code"], json!("store_unavailable"));
    let reason = answer["error"]["reason"].as_str().unwrap();
    assert!(
        !reason.contains("hunter2") && reason.contains("unreachable"),
        "{reason}"
    );
    assert_eq!(answer["error"]["message"].as_str().unwrap(), reason);
    let (code, answer, _) = cli(None, &["list_ready"]);
    assert_eq!(
        (code, &answer["error"]["code"]),
        (1, &json!("store_unavailable"))
    );
    let (code, answer, stderr) = cli(Some(memory_board), &["help"]);
    assert_eq!((code, answer), (0, Value::Null));
    assert!(stderr.starts_with("usage: quecto board"), "{stderr}");
}
