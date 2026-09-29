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
