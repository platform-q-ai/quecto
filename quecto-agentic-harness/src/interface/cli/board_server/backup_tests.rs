use super::*;
#[test]
fn backup_options_admit_only_complete_known_pairs() {
    for values in [
        vec![],
        vec!["--to"],
        vec!["--to", "out", "--to", "other"],
        vec!["--to", "out", "--source", "somewhere"],
        vec!["--to", "--board-id"],
        vec!["--to", "out", "--board-id", "../escape"],
    ] {
        let args = values.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(parse(&args).is_err());
    }
    let options =
        parse(&["--board-id", "0123456789abcdef", "--to", "./my snapshots"].map(str::to_owned))
            .unwrap();
    assert_eq!(options.board.as_deref(), Some("0123456789abcdef"));
    assert_eq!(options.to, PathBuf::from("./my snapshots"));
}
#[test]
fn backup_invalid_options_never_construct_graph() {
    fn trap() -> Result<BackupBoard, String> {
        panic!("bad argv must precede composition")
    }
    let context = CliContext {
        board_backup: Some(trap),
        ..Default::default()
    };
    assert_eq!(
        run(&context, &[], &mut String::new(), &mut String::new()),
        1
    );
}
