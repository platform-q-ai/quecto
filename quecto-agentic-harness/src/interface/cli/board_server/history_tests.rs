use super::*;
fn args() -> Vec<String> {
    [
        "--repo",
        "/scratch/destination",
        "--board-id",
        "0123456789abcdef",
        "--to-branch",
        "rollback",
        "--format",
        "3",
        "--dry-run",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
#[test]
fn export_parser_is_allowlisted_and_help_needs_no_builder() {
    let (environment, dry) = parse(&args()).unwrap();
    assert_eq!(environment.repo, PathBuf::from("/scratch/destination"));
    assert_eq!(environment.board.as_str(), "0123456789abcdef");
    assert_eq!(environment.target, "refs/heads/rollback");
    assert!(dry);
    for extra in [
        vec!["--dry-run"],
        vec!["--repo"],
        vec!["--remote", "-unsafe"],
        vec!["--socket", "/scratch/socket"],
    ] {
        let mut command = args();
        command.extend(extra.into_iter().map(str::to_owned));
        assert!(parse(&command).is_err());
    }
    let (mut out, mut err) = (String::new(), String::new());
    assert_eq!(
        run(
            &CliContext::default(),
            &["--help".into()],
            &mut out,
            &mut err
        ),
        0
    );
    assert!(out.contains("board-server export"));
    assert!(err.is_empty());
}
#[test]
fn export_dispatch_uses_only_the_injected_export_builder() {
    fn build(e: HistoryCliEnvironment) -> Result<HistoryCliHandles, String> {
        assert_eq!(e.board.as_str(), "0123456789abcdef");
        assert_eq!(e.target, "refs/heads/rollback");
        Err("injected offline export graph".into())
    }
    let ctx = CliContext {
        board_history: Some(build),
        ..Default::default()
    };
    let mut command = vec!["export".into()];
    command.extend(args());
    let (mut out, mut err) = (String::new(), String::new());
    assert_eq!(
        super::super::cmd_board_server(&ctx, &command, &mut out, &mut err),
        1
    );
    assert!(err.contains("injected offline export graph"));
    assert!(out.is_empty());
}
