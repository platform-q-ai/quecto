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
