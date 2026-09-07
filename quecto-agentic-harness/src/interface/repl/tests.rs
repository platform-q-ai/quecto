use super::run_repl;
use std::io::{BufRead, Cursor};

fn run(input: &str) -> (String, Vec<Vec<String>>) {
    let mut output = Vec::new();
    let mut calls = Vec::new();
    let code = run_repl(Cursor::new(input), &mut output, true, |args, _reader| {
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

#[test]
fn non_tty_repl_returns_the_last_failed_command_status() {
    let mut output = Vec::new();
    let code = run_repl(
        Cursor::new("auth bogus\n"),
        &mut output,
        false,
        |_args, _reader| (String::new(), "invalid\n".into(), 7),
    );
    assert_eq!(code, 7);
}

#[test]
fn help_documents_the_actionable_models_discover_syntax() {
    let (output, _) = run("help\nexit\n");
    assert!(output.contains("models discover <provider-key>"));
}

#[test]
fn delegated_commands_can_consume_follow_up_input_from_the_repl_reader() {
    let mut output = Vec::new();
    let mut consumed = String::new();
    let code = run_repl(
        Cursor::new("auth login\n1\nexit\n"),
        &mut output,
        false,
        |args, reader| {
            assert_eq!(args, ["auth", "login"]);
            reader.read_line(&mut consumed).unwrap();
            ("logged in\n".into(), String::new(), 0)
        },
    );
    assert_eq!(code, 0);
    assert_eq!(consumed, "1\n");
    assert_eq!(String::from_utf8(output).unwrap(), "logged in\n");
}
