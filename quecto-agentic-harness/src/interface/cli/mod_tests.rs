use super::*;

fn args(command: &str) -> Vec<String> {
    std::iter::once("quecto".to_owned())
        .chain(command.split_whitespace().map(str::to_owned))
        .collect()
}

#[test]
fn live_cli_commands_keep_help_version_and_unknown_dispatch() {
    let help = run_with_output(args("help"), &CliContext::default());
    assert_eq!(help.exit_code, 0);
    assert!(help.stdout.contains("Usage: quecto [command]"));
    assert!(help.stdout.contains("agent"));

    let version = run_with_output(args("--version"), &CliContext::default());
    assert_eq!(version.exit_code, 0);
    assert!(version.stdout.contains(env!("CARGO_PKG_VERSION")));

    let unknown = run_with_output(args("not-a-command"), &CliContext::default());
    assert_eq!(unknown.exit_code, 1);
    assert!(unknown.stderr.contains("Unknown command: not-a-command"));
}

#[test]
fn real_run_dispatches_non_repl_commands() {
    assert_eq!(run(args("version")), 0);
    assert_eq!(run(args("definitely-not-a-command")), 1);
}
