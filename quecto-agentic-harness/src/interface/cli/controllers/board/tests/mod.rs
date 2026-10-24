use super::*;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::use_cases::QueryBoard;
use crate::composition::bootstrap::project_board_operations::fixtures::{QueryListings, assembled};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::run;
use serde_json::json;
use std::sync::Arc;

fn args(line: &[&str]) -> Vec<String> {
    line.iter().map(|arg| (*arg).to_string()).collect()
}

fn built(line: &[&str], stdin: &str) -> Result<Value, RequestError> {
    let line = read_line(&args(line)).map_err(|refused| refused.error)?;
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
        (&["renew", "--task"][..], "", "flags"),
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
    let broken = request(&line, || {
        Err(InputError::Read(std::io::Error::other("the pipe broke")))
    })
    .unwrap_err();
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
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
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
        crate::interface::cli::controllers::config_flag::extract_config_flag(&global),
        Ok(Some(PathBuf::from("c.json")))
    );
    assert_eq!(
        crate::interface::cli::controllers::config_flag::strip_global_config_flag(&global),
        args(&["quecto", "board", "x", "--config", "y"])
    );
}

/// A flag with no value, or one the op does not take, is answered on a
/// fixed field in fixed words: its text is never echoed back.
#[test]
fn a_stray_flag_is_never_echoed_back() {
    for line in [
        &["renew", "--<script>"][..],
        &["renew", "--<script>", "x"][..],
    ] {
        let (code, answer, _) = cli(Some(never_built), line);
        assert_eq!(code, 1, "{line:?}");
        assert_eq!(answer["error"]["field"], json!("flags"), "{answer}");
        assert!(!answer.to_string().contains("script"), "{answer}");
    }
    let (_, answer, _) = cli(Some(never_built), &["<script>", "--task", "a"]);
    assert_eq!(
        (
            &answer["error"]["field"],
            &answer["error"]["problem"],
            &answer["error"]["message"]
        ),
        (
            &json!("op"),
            &json!("is not an op of the board"),
            &json!("op: is not an op of the board")
        )
    );
}

/// Epic #2481 T3 review round 1: `--input` with field flags beside it is
/// the request as a whole given wrong, recorded as `request`.
#[test]
fn input_beside_field_flags_is_recorded_as_the_request() {
    let error = built(&["create", "--input", "{}", "--title", "A"], "").unwrap_err();
    assert_eq!(error.field, "input", "the answer is unchanged");
    let fault = error.fault;
    assert_eq!(
        (
            fault.field.as_str(),
            fault.fault.as_str(),
            fault.unexpected_count
        ),
        ("request", "invalid_value", None)
    );
}

/// Review round 2: `--input -` over the bound, or not UTF-8, is recorded
/// as the shared path records a request's text: `request`, `too_large`
/// or `not_json`; a pipe that breaks is the request as a whole, unread.
#[test]
fn stdin_input_failures_are_recorded_as_the_requests() {
    let recorded = |error: RequestError| (error.fault.field.as_str(), error.fault.fault.as_str());
    let big = "x".repeat(MAX_REQUEST_BYTES + 1);
    let too_large = built(&["--input", "-"], &big).unwrap_err();
    assert_eq!(recorded(too_large), ("request", "too_large"));
    let line = read_line(&args(&["--input", "-"])).unwrap();
    let not_utf8 = request(&line, || bounded(&[0xff, 0xfe][..])).unwrap_err();
    assert_eq!(not_utf8.problem, "could not be read: it is not UTF-8");
    assert_eq!(recorded(not_utf8), ("request", "not_json"));
    let broken = request(&line, || Err(InputError::Read(std::io::Error::other("x")))).unwrap_err();
    assert_eq!(recorded(broken), ("request", "invalid_value"));
}

/// Epic #2481 T3: a caller names itself by allowlist; its text is never
/// recorded.
#[test]
fn the_client_is_named_by_allowlist() {
    use std::ffi::OsStr;
    assert_eq!(client(None), Client::Cli);
    // The agent tool's own when unset (Q10), the relay's when it names
    // itself in the harness it launched.
    use crate::interface::tools::project_board::recording::named_client;
    assert_eq!(named_client(None, Client::Agent), Client::Agent);
    assert_eq!(
        named_client(Some(OsStr::new("pocket-relay")), Client::Agent),
        Client::PocketRelay
    );
    assert_eq!(
        named_client(Some(OsStr::new("x")), Client::Agent),
        Client::Other
    );
    assert_eq!(
        client(Some(OsStr::new("pocket-relay"))),
        Client::PocketRelay
    );
    for other in [
        "pocket_relay",
        "Pocket-Relay",
        "pocket-relay ",
        "",
        "cli",
        "agent",
    ] {
        assert_eq!(client(Some(OsStr::new(other))), Client::Other, "{other:?}");
    }
}

/// `quecto board` run under `base`, recording as composition builds it.
fn recorded_cli(base: &std::path::Path, line: &[&str]) -> Value {
    let ctx = CliContext {
        project_board: Some(memory_board),
        board_cli_recording: Some(CliRecordingBuilder(
            crate::composition::bootstrap::project_board::cli_board_recording,
        )),
        base_dir: Some(base.to_path_buf()),
        ..CliContext::default()
    };
    let (mut stdout, mut stderr) = (String::new(), String::new());
    cmd_board(&ctx, &args(line), &mut stdout, &mut stderr);
    serde_json::from_str(stdout.trim()).unwrap()
}

/// The `board_op` lines of the day's CLI logs under `base`.
fn cli_logs(base: &std::path::Path) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(base.join("audit")) else {
        return vec![];
    };
    let mut lines = vec![];
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("board-cli_") && name.ends_with(".jsonl"),
            "{name}"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        let read = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap());
        lines.extend(read.filter(|line| line["event"] == "board_op"));
    }
    lines
}

/// Owner condition (2026-10-12): the CLI writes its day's log only while
/// the owner's global config switches the event log on; one record per
/// call that answers, a refusal before the board was read included.
#[test]
fn the_cli_writes_its_daily_log_only_while_the_switch_is_on() {
    let off = tempfile::tempdir().unwrap();
    let answer = recorded_cli(off.path(), &["list"]);
    assert_eq!(answer["ok"], json!(true), "{answer}");
    assert!(
        !off.path().join("audit").exists(),
        "nothing opened while off"
    );
    let on = tempfile::tempdir().unwrap();
    let config = json!({"telemetry": {"event_log": {"enabled": true}}});
    std::fs::write(on.path().join("config.json"), config.to_string()).unwrap();
    recorded_cli(on.path(), &["list"]);
    recorded_cli(
        on.path(),
        &["renew", "--task", "a", "--holder", "Secret Holder"],
    );
    let lines = cli_logs(on.path());
    let got: Vec<(&Value, &Value, &Value)> = lines
        .iter()
        .map(|line| (&line["op"], &line["front_end"], &line["outcome"]))
        .collect();
    assert_eq!(
        got,
        [
            (&json!("list"), &json!("cli"), &json!("ok")),
            (&json!("renew"), &json!("cli"), &json!("refused")),
        ]
    );
    assert_eq!(lines[0]["listed"], json!(1), "{}", lines[0]);
    assert_eq!(
        (&lines[1]["field"], &lines[1]["fault"]),
        (&json!("other"), &json!("unexpected"))
    );
    assert!(!lines[1].to_string().contains("Secret"), "{}", lines[1]);
}

/// Review round 1, the property test at the CLI's entry point: random
/// titles, keys and words given as flags, as `--input` or as stray
/// arguments never appear in a record.
#[test]
fn no_request_text_reaches_a_record_through_the_cli() {
    use crate::interface::tools::project_board::recording::tests::outside_text;
    use rand::{Rng, SeedableRng};
    let seed: u64 = rand::thread_rng().r#gen();
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let on = tempfile::tempdir().unwrap();
    let config = json!({"telemetry": {"event_log": {"enabled": true}}});
    std::fs::write(on.path().join("config.json"), config.to_string()).unwrap();
    let mut texts = vec![];
    let mut calls = 0;
    for case in 0..8 {
        let (title, key) = (outside_text(&mut rng), outside_text(&mut rng));
        let flag = format!("--{key}");
        let input = json!({"op": "create", "task": format!("t-{case}"), "title": title, key.clone(): title}).to_string();
        let shown = json!({"task": title}).to_string();
        let task = format!("u-{case}");
        let lines: [&[&str]; 6] = [
            &[
                "create",
                "--task",
                &task,
                "--title",
                &title,
                "--description",
                &key,
            ],
            &["list", &flag, &title],
            &["create", "--input", &input],
            &["show", "--input", &shown],
            &[&key, "--task", &title],
            &["renew", &title, &key],
        ];
        for line in lines {
            recorded_cli(on.path(), line);
            calls += 1;
        }
        texts.extend([title, key]);
    }
    let lines = cli_logs(on.path());
    assert_eq!(lines.len(), calls, "one record per call");
    for line in lines.iter().map(Value::to_string) {
        for text in &texts {
            assert!(
                !line.contains(text.as_str()),
                "seed {seed}: {text:?} reached {line}"
            );
        }
    }
}

/// Review round 2: a command line refused as it is read records the op it
/// named before the refusal; the answer still names none.
#[test]
fn a_line_refused_as_it_is_read_records_the_op_it_named() {
    let on = tempfile::tempdir().unwrap();
    let config = json!({"telemetry": {"event_log": {"enabled": true}}});
    std::fs::write(on.path().join("config.json"), config.to_string()).unwrap();
    let answer = recorded_cli(on.path(), &["renew", "--task"]);
    assert_eq!(answer["op"], Value::Null, "{answer}");
    recorded_cli(on.path(), &["--task"]);
    let ops: Vec<Value> = cli_logs(on.path())
        .iter()
        .map(|line| line["op"].clone())
        .collect();
    assert_eq!(ops, [json!("renew"), json!("unknown")]);
}

/// S4c: `quecto board start` is refused in the owner's words before
/// anything is built: only an agent session can host a swarm.
#[test]
fn the_command_line_takes_every_op_but_start() {
    use crate::interface::tools::project_board::request::OPS;
    let table: Vec<&str> = OPS.iter().map(|(op, _)| *op).collect();
    let mut taken: Vec<&str> = CLI_OPS.to_vec();
    taken.push("start");
    taken.sort_unstable();
    let mut every = table.clone();
    every.sort_unstable();
    assert_eq!(
        taken, every,
        "each op of the table is taken or refused by name"
    );
    assert!(not_taken("start").starts_with("start needs an agent session"));
    assert_eq!(
        not_taken("other"),
        "other is not taken by quecto board: ask a quecto agent"
    );
}

#[test]
fn start_is_refused_on_the_command_line() {
    let (code, answer, _) = cli(
        Some(never_built),
        &["start", "--task", "a", "--plan-version", "1"],
    );
    assert_eq!(
        (
            code,
            &answer["op"],
            &answer["error"]["code"],
            &answer["error"]["problem"]
        ),
        (
            1,
            &json!("start"),
            &json!("invalid_request"),
            &json!("start needs an agent session: ask a quecto agent, or use Pocket")
        )
    );
    // The start steps are the agent tool's: the command line's refusal
    // carries none (review round 1, #2567).
    assert!(answer["error"].get("steps").is_none(), "{answer}");
}

/// `--summary` gives each listed task its summary, on every listing op,
/// with flags or `--input` alike; without it a listing is as before.
#[test]
fn summary_is_the_command_lines_opt_in_for_listings() {
    let none = json!({"plan": null, "run": null, "may_be_incomplete": false});
    for line in [
        &["list", "--summary"][..],
        &["list-ready", "--summary"][..],
        &["--summary", "list", "--status", "ready"][..],
        &["list", "--input", r#"{"status": "ready"}"#, "--summary"][..],
    ] {
        let (code, answer, _) = cli(Some(memory_board), line);
        assert_eq!(code, 0, "{line:?}: {answer}");
        assert_eq!(answer["tasks"][0]["id"], json!("a"), "{line:?}: {answer}");
        assert_eq!(answer["tasks"][0]["summary"], none, "{line:?}: {answer}");
        assert_eq!(answer.get("summary_skipped"), None, "{line:?}: {answer}");
    }
    let (code, answer, _) = cli(Some(memory_board), &["list-takeable", "--summary"]);
    assert_eq!((code, &answer["tasks"]), (0, &json!([])), "{answer}");
    for line in [&["list"][..], &["list-ready"][..], &["list-takeable"][..]] {
        let (code, answer, _) = cli(Some(memory_board), line);
        assert_eq!(code, 0, "{line:?}: {answer}");
        // A takeable listing's clock note is its own, as ever.
        let mut keys: Vec<&str> = answer
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .filter(|key| *key != "clock")
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["head", "ok", "op", "quarantined", "tasks"],
            "{line:?}"
        );
        let tasks = answer["tasks"].as_array().unwrap();
        assert!(
            tasks.iter().all(|task| task.get("summary").is_none()),
            "{answer}"
        );
    }
}

/// `--summary` takes no value, is given once, and only of a listing: of
/// another op it is refused before anything is built.
#[test]
fn summary_is_refused_beside_another_op_or_twice() {
    for line in [
        &["show", "--task", "a", "--summary"][..],
        &["renew", "--summary", "--task", "a"][..],
        &["show", "--input", r#"{"task": "a"}"#, "--summary"][..],
    ] {
        let (code, answer, _) = cli(Some(never_built), line);
        assert_eq!(code, 1, "{line:?}: {answer}");
        assert_eq!(
            answer["error"]["code"],
            json!("invalid_request"),
            "{answer}"
        );
        assert_eq!(answer["error"]["field"], json!("summary"), "{answer}");
    }
    let (code, answer, _) = cli(Some(never_built), &["list", "--summary", "--summary"]);
    assert_eq!(code, 1, "{answer}");
    assert_eq!(
        (&answer["error"]["field"], &answer["error"]["problem"]),
        (&json!("summary"), &json!("is given twice"))
    );
}

mod moved;
