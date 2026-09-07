use super::run_repl;
use std::io::Cursor;

fn run(input: &str) -> (String, Vec<Vec<String>>) {
    let mut output = Vec::new();
    let mut calls = Vec::new();
    let code = run_repl(Cursor::new(input), &mut output, true, |args| {
        calls.push(args);
        ("configured\n".into(), String::new(), 0)
    });
    assert_eq!(code, 0);
    (String::from_utf8(output).unwrap(), calls)
}

#[test]
fn help_describes_only_setup_and_configuration_scope() {
    let (output, calls) = run("help\nexit\n");
    assert!(output.contains("Setup and configuration commands"));
    assert!(output.contains("auth login"));
    assert!(output.contains("use `quecto agent` or `quecto-tui`"));
    assert!(!output.contains("/spawn"));
    assert!(!output.contains("/clear"));
    assert!(calls.is_empty());
}

#[test]
fn arbitrary_prompts_and_removed_agent_commands_are_not_dispatched() {
    let (output, calls) = run("tell me a joke\n/spawn child\n/agent list\nexit\n");
    assert_eq!(output.matches("Unsupported REPL command").count(), 3);
    assert!(calls.is_empty());
}

#[test]
fn supported_configuration_commands_delegate_to_cli_adapter() {
    let (output, calls) = run("auth status\nstatus\nmodels list\nexit\n");
    assert_eq!(output.matches("configured").count(), 3);
    assert_eq!(calls[0], ["auth", "status"]);
    assert_eq!(calls[1], ["status"]);
    assert_eq!(calls[2], ["models", "list"]);
}
