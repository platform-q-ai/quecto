use super::*;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::{QueryListings, assembled};
use crate::application::project_board::use_cases::QueryBoard;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use serde_json::json;
use std::sync::Arc;

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
        (&["renew", "extra"][..], "", "arguments"),
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
    let big = "x".repeat(MAX_REQUEST_BYTES + 1);
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

/// A board with one ready task `a`, in memory, built afresh each run; it
/// says which access it was built for.
fn memory_board(environment: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    if environment.remote == "no-committer" && environment.access == Access::Write {
        return Err(BuildError::NoCommitter(
            "user.name is not configured".into(),
        ));
    }
    let store = MemoryBoardStore::default();
    use crate::application::project_board::ports::ProjectBoardStore;
    store.bootstrap().unwrap();
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    let created = run(
        &operations,
        r#"{"op": "create", "task": "a", "title": "A", "status": "ready"}"#,
    );
    assert_eq!(created["ok"], json!(true), "{created}");
    Ok(ProjectBoardHandles { operations })
}

fn unbuildable(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    Err(BuildError::Unavailable(
        "https://user:hunter2@example.com/repo is unreachable".into(),
    ))
}

fn never_built(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    panic!("a request refused while being read builds nothing")
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
    let (code, answer, _) = cli(Some(memory_board), &["list", "--remote", "no-committer"]);
    assert_eq!((code, &answer["ok"]), (0, &json!(true)));
    let (code, answer, _) = cli(
        Some(memory_board),
        &["renew", "--task", "a", "--remote", "no-committer"],
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

#[test]
fn input_that_is_not_utf8_is_refused_and_input_at_the_bound_is_read() {
    let broken = bounded(&[0xff, 0xfe][..]).unwrap_err();
    assert!(broken.to_string().contains("not UTF-8"), "{broken}");
    let exact = " ".repeat(MAX_REQUEST_BYTES);
    assert_eq!(bounded(exact.as_bytes()).unwrap().len(), MAX_REQUEST_BYTES);
}

/// An op is echoed back only when it is one, as on the tool's path.
#[test]
fn an_unknown_op_is_not_echoed_back() {
    for line in [
        &["no-such-op", "--task", "a"][..],
        &["<script>", "--x", "y"][..],
    ] {
        let (code, answer, _) = cli(Some(never_built), line);
        assert_eq!((code, &answer["op"]), (1, &Value::Null), "{line:?}");
    }
    let (_, answer, _) = cli(Some(never_built), &["renew", "--holder", "x"]);
    assert_eq!(answer["op"], json!("renew"));
}

#[test]
fn the_command_lines_own_flags_are_given_once_with_a_value() {
    let (twice, empty) = ("is given twice", "needs a value");
    for (line, field, problem) in [
        (&["list", "--repo", "a", "--repo", "b"][..], "repo", twice),
        (
            &["list", "--remote", "a", "--remote", "b"][..],
            "remote",
            twice,
        ),
        (
            &["list", "--input", "{}", "--input", "{}"][..],
            "input",
            twice,
        ),
        (&["list", "--repo", ""][..], "repo", empty),
        (&["list", "--remote", ""][..], "remote", empty),
        (&["list", "--input", ""][..], "input", empty),
    ] {
        let (code, answer, _) = cli(Some(never_built), line);
        assert_eq!(code, 1, "{line:?}");
        assert_eq!(
            answer["error"]["code"],
            json!("invalid_request"),
            "{line:?}"
        );
        assert_eq!(answer["error"]["field"], json!(field), "{line:?}");
        assert_eq!(answer["error"]["problem"], json!(problem), "{line:?}");
    }
}

/// A request the parser refuses (a value of the wrong kind, a field
/// missing from `--input`) builds nothing either.
#[test]
fn a_request_the_parser_refuses_builds_nothing() {
    for (line, field) in [
        (&["move", "--task", "a", "--to", "nowhere"][..], "to"),
        (&["--input", r#"{"op": "renew"}"#][..], "task"),
    ] {
        let (code, answer, _) = cli(Some(never_built), line);
        assert_eq!(
            (code, &answer["error"]["field"]),
            (1, &json!(field)),
            "{line:?}"
        );
    }
}

/// `board`'s own arguments are its request's: a `--config` among them is
/// a field, answered as JSON, never the global flag.
#[test]
fn a_config_flag_after_board_is_the_requests() {
    let ctx = CliContext {
        project_board: Some(never_built),
        ..CliContext::default()
    };
    let line = ["quecto", "board", "create", "--title", "--config"];
    let output = crate::interface::cli::run_with_output(args(&line), &ctx);
    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stderr, "");
    let answer: Value = serde_json::from_str(output.stdout.trim()).unwrap();
    assert_eq!(answer["error"]["field"], json!("task"), "{answer}");
    let global = args(&[
        "quecto", "--config", "c.json", "board", "x", "--config", "y",
    ]);
    assert_eq!(
        super::super::config_flag::extract_config_flag(&global),
        Ok(Some(PathBuf::from("c.json")))
    );
    assert_eq!(
        super::super::config_flag::strip_global_config_flag(&global),
        args(&["quecto", "board", "x", "--config", "y"])
    );
}
