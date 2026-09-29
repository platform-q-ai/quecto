use std::io::Write;
use std::process::{Command, Stdio};

use super::{MOCK_CLAUDE_PROGRAM, write_mock_claude};

fn run(scenario: &str, input: &str) -> (String, String) {
    let root = tempfile::tempdir().unwrap();
    let scenario_path = root.path().join("scenario.jsonl");
    std::fs::write(&scenario_path, scenario).unwrap();
    let mock = write_mock_claude(&root.path().join("bin"), &scenario_path);
    let mut child = Command::new(mock.bin_dir.join(MOCK_CLAUDE_PROGRAM))
        .arg("-p")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(mock.recorded("arg"), vec!["-p".to_string()]);
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn each_input_line_replays_the_next_turn_up_to_its_result() {
    let scenario = concat!(
        "{\"type\": \"system\", \"subtype\": \"init\"}\n",
        "{\"type\": \"result\", \"n\": 1}\n",
        "{\"type\": \"assistant\"}\n",
        "{\"type\":\"result\", \"n\": 2}\n",
    );
    let (stdout, _) = run(scenario, "one\ntwo\n");
    assert_eq!(stdout, scenario);
    let (first_only, _) = run(scenario, "one\n");
    assert_eq!(
        first_only,
        "{\"type\": \"system\", \"subtype\": \"init\"}\n{\"type\": \"result\", \"n\": 1}\n"
    );
}

#[test]
fn stderr_directives_write_to_stderr_not_stdout() {
    let scenario = "@stderr hello\n@stderr-fill 2050\n{\"type\": \"result\"}\n";
    let (stdout, stderr) = run(scenario, "go\n");
    assert_eq!(stdout, "{\"type\": \"result\"}\n");
    assert!(stderr.starts_with("hello\n"), "{stderr:?}");
    assert_eq!(stderr.len(), "hello\n".len() + 2050);
}

#[test]
fn stall_and_stubborn_directives_are_never_emitted() {
    // `@stubborn` is not replayed here: a stubborn mock outlives its
    // input, which only the supervisor's KILL ends (process tests).
    let scenario = "@stall-input 0\n{\"type\": \"result\"}\n";
    let (stdout, _) = run(scenario, "go\n");
    assert_eq!(stdout, "{\"type\": \"result\"}\n");
}

#[test]
fn the_start_record_names_the_pid_group_and_private_dir_modes() {
    let root = tempfile::tempdir().unwrap();
    let scenario = root.path().join("scenario.jsonl");
    std::fs::write(&scenario, "").unwrap();
    let mock = write_mock_claude(&root.path().join("bin"), &scenario);
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let output = Command::new(mock.bin_dir.join(MOCK_CLAUDE_PROGRAM))
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", root.path().join("absent"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let pid = mock.recorded("pid");
    assert_eq!(pid.len(), 1, "{pid:?}");
    assert!(pid[0].parse::<u32>().is_ok(), "{pid:?}");
    assert!(mock.recorded("pgid")[0].parse::<u32>().is_ok());
    assert!(
        mock.recorded("home_mode")[0].starts_with('d'),
        "{:?}",
        mock.recorded("home_mode")
    );
    assert_eq!(mock.recorded("config_mode"), vec![String::new()]);
    assert!(mock.recorded("grandchild").is_empty());
}

#[test]
fn printf_writes_raw_bytes_and_record_marks_progress_in_its_turn() {
    let root = tempfile::tempdir().unwrap();
    let scenario = root.path().join("scenario.jsonl");
    std::fs::write(
        &scenario,
        "@printf \\377\\376{}\\n\n@record reached\n{\"type\": \"result\"}\n",
    )
    .unwrap();
    let mock = write_mock_claude(&root.path().join("bin"), &scenario);
    let mut child = Command::new(mock.bin_dir.join(MOCK_CLAUDE_PROGRAM))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"go\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"\xff\xfe{}\n{\"type\": \"result\"}\n".to_vec()
    );
    assert_eq!(mock.recorded("record"), vec!["reached".to_string()]);
}

/// One stream-json user line under `uuid`, as the adapter writes it.
fn user(uuid: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"uuid\":\"{uuid}\",\"message\":{{\"role\":\"user\",\"content\":[]}}}}\n"
    )
}

/// An interrupt control request, with or without `cancel_queued`.
fn interrupt(request_id: &str, cancel_queued: bool) -> String {
    format!(
        "{{\"type\":\"control_request\",\"request_id\":\"{request_id}\",\"request\":{{\"subtype\":\"interrupt\"{}}}}}\n",
        match cancel_queued {
            true => ",\"cancel_queued\":true",
            false => "",
        }
    )
}

fn control_response(request_id: &str, still_queued: &str, cancelled: &str) -> String {
    format!(
        "{{\"type\":\"control_response\",\"response\":{{\"subtype\":\"success\",\"request_id\":\"{request_id}\",\"response\":{{\"still_queued\":[{still_queued}],\"cancelled\":[{cancelled}]}}}}}}\n"
    )
}

/// #2287 review round 2 (L9): user messages written while a turn runs are
/// folded into it, as claude folds a queued message between tool rounds:
/// its one result lists every uuid it consumed (`@UUIDS@`), the turn's
/// own first.
#[test]
fn messages_written_mid_turn_fold_into_the_running_turn() {
    let scenario = concat!(
        "{\"type\":\"system\",\"subtype\":\"init\"}\n",
        "@await-steers 2\n",
        "{\"type\":\"result\",\"user_message_uuid\":\"@UUID@\",\"user_message_uuids\":[@UUIDS@]}\n",
        "{\"type\":\"result\",\"n\":2,\"user_message_uuids\":[@UUIDS@]}\n",
    );
    let input = [user("a"), user("b"), user("c"), user("d")].concat();
    let (stdout, _) = run(scenario, &input);
    assert_eq!(
        stdout,
        concat!(
            "{\"type\":\"system\",\"subtype\":\"init\"}\n",
            "{\"type\":\"result\",\"user_message_uuid\":\"a\",\"user_message_uuids\":[\"a\",\"b\",\"c\"]}\n",
            "{\"type\":\"result\",\"n\":2,\"user_message_uuids\":[\"d\"]}\n",
        )
    );
}

/// The CLI names at most 64 user turns in one result.
#[test]
fn a_result_names_at_most_64_user_turns() {
    let scenario = "@await-steers 70\n{\"type\":\"result\",\"user_message_uuids\":[@UUIDS@]}\n";
    let input: String = (0..71).map(|n| user(&format!("m{n}"))).collect();
    let (stdout, _) = run(scenario, &input);
    let result: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let named = result["user_message_uuids"].as_array().unwrap();
    assert_eq!(named.len(), 64);
    assert_eq!(named[0], "m0");
    assert_eq!(named[63], "m63");
}

/// An interrupt with `cancel_queued` withdraws the user messages queued
/// behind the running turn, lists them as cancelled and runs none of them;
/// under `@await-interrupt` only a control request is answered as one.
#[test]
fn an_interrupt_withdraws_the_queued_messages_it_cancels() {
    let scenario = concat!(
        "@await-interrupt\n",
        "{\"type\":\"result\",\"terminal_reason\":\"aborted_streaming\",\"user_message_uuids\":[@UUIDS@]}\n",
        "{\"type\":\"result\",\"n\":2,\"user_message_uuids\":[@UUIDS@]}\n",
    );
    let input = [user("a"), user("b"), user("c"), interrupt("r1", true)].concat();
    let (stdout, _) = run(scenario, &input);
    assert_eq!(
        stdout,
        [
            control_response("r1", "", "\"b\",\"c\""),
            "{\"type\":\"result\",\"terminal_reason\":\"aborted_streaming\",\"user_message_uuids\":[\"a\"]}\n".to_string(),
        ]
        .concat()
    );
}

/// Without `cancel_queued` the queued messages survive the interrupt
/// (`still_queued`) and run, together, as the next turn.
#[test]
fn an_interrupt_without_cancel_queued_leaves_the_queue_to_run() {
    let scenario = concat!(
        "@await-interrupt\n",
        "{\"type\":\"result\",\"terminal_reason\":\"aborted_streaming\",\"user_message_uuids\":[@UUIDS@]}\n",
        "{\"type\":\"result\",\"n\":2,\"user_message_uuids\":[@UUIDS@]}\n",
    );
    let input = [user("a"), user("b"), interrupt("r1", false)].concat();
    let (stdout, _) = run(scenario, &input);
    assert_eq!(
        stdout,
        [
            control_response("r1", "\"b\"", ""),
            "{\"type\":\"result\",\"terminal_reason\":\"aborted_streaming\",\"user_message_uuids\":[\"a\"]}\n".to_string(),
            "{\"type\":\"result\",\"n\":2,\"user_message_uuids\":[\"b\"]}\n".to_string(),
        ]
        .concat()
    );
}

/// An interrupt while idle stops nothing and withdraws nothing.
#[test]
fn an_idle_interrupt_is_answered_with_nothing_withdrawn() {
    let scenario = "{\"type\":\"result\",\"user_message_uuids\":[@UUIDS@]}\n";
    let input = [interrupt("r0", true), user("a")].concat();
    let (stdout, _) = run(scenario, &input);
    assert_eq!(
        stdout,
        [
            control_response("r0", "", ""),
            "{\"type\":\"result\",\"user_message_uuids\":[\"a\"]}\n".to_string(),
        ]
        .concat()
    );
}

/// #2287 review round 3: the messages an interrupt left queued run as one
/// merged batch, whose turn is the LAST member's (`user_message_uuid`), as
/// claude runs "several messages sent close together".
#[test]
fn a_merged_batch_is_the_last_members_turn() {
    let scenario = concat!(
        "@await-interrupt\n",
        "{\"type\":\"result\",\"user_message_uuid\":\"@UUID@\",\"user_message_uuids\":[@UUIDS@]}\n",
        "{\"type\":\"result\",\"user_message_uuid\":\"@UUID@\",\"user_message_uuids\":[@UUIDS@]}\n",
    );
    let input = [user("a"), user("b"), user("c"), interrupt("r1", false)].concat();
    let (stdout, _) = run(scenario, &input);
    assert_eq!(
        stdout,
        [
            control_response("r1", "\"b\",\"c\"", ""),
            "{\"type\":\"result\",\"user_message_uuid\":\"a\",\"user_message_uuids\":[\"a\"]}\n"
                .to_string(),
            "{\"type\":\"result\",\"user_message_uuid\":\"c\",\"user_message_uuids\":[\"b\",\"c\"]}\n"
                .to_string(),
        ]
        .concat()
    );
}

/// #2287 review round 3: claude's collector keeps a merged batch's first
/// 64 uuids and, past that, overwrites slot 63 with the turn's own (the
/// last member's), so the list always names the turn.
#[test]
fn a_merged_batch_past_64_keeps_its_own_uuid_in_slot_63() {
    let scenario = concat!(
        "@await-interrupt\n",
        "{\"type\":\"result\",\"n\":1}\n",
        "{\"type\":\"result\",\"user_message_uuid\":\"@UUID@\",\"user_message_uuids\":[@UUIDS@]}\n",
    );
    let queued: String = (1..=70).map(|n| user(&format!("m{n}"))).collect();
    let input = [user("m0"), queued, interrupt("r1", false)].concat();
    let (stdout, _) = run(scenario, &input);
    let last = stdout.lines().last().unwrap();
    let result: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(result["user_message_uuid"], "m70");
    let named = result["user_message_uuids"].as_array().unwrap();
    assert_eq!(named.len(), 64);
    assert_eq!(named[0], "m1");
    assert_eq!(named[62], "m63");
    assert_eq!(named[63], "m70", "the turn's own, in slot 63");
}
