use super::*;

#[test]
fn the_restart_runs_the_same_program_with_the_same_arguments() {
    let command = session_restart_command(
        PathBuf::from("/usr/bin/quecto-tui"),
        ["--model".into(), "m".into(), "--persist".into()],
    );
    assert_eq!(command.get_program(), "/usr/bin/quecto-tui");
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, ["--model", "m", "--persist"]);
}
