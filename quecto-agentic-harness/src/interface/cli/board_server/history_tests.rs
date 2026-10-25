use super::*;
fn base() -> &'static std::path::Path {
    std::path::Path::new("/scratch/base")
}
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
    let (environment, dry) = parse(&args(), base()).unwrap();
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
        assert!(parse(&command, base()).is_err());
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

#[test]
fn import_parser_help_and_injected_graph_are_complete_and_allowlisted() {
    let args: Vec<String> = [
        "--repo",
        "/scratch/source",
        "--board-id",
        "0123456789abcdef",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let e = parse_import(&args, base()).unwrap();
    assert_eq!(e.repo, PathBuf::from("/scratch/source"));
    assert_eq!(e.board.as_str(), "0123456789abcdef");
    assert_eq!(e.remote, "origin");
    for extra in [
        vec!["--dry-run"],
        vec!["--to-branch", "rollback"],
        vec!["--format", "3"],
        vec!["--remote", "-unsafe"],
        vec!["--repo", "again"],
    ] {
        let mut bad = args.clone();
        bad.extend(extra.into_iter().map(str::to_owned));
        assert!(parse_import(&bad, base()).is_err());
    }
    for id in ["../escape", "0123456789abcdeg", "0123456789abcdef0", ""] {
        let mut bad = args.clone();
        bad[3] = id.into();
        assert!(parse_import(&bad, base()).is_err());
    }
    let (mut out, mut err) = (String::new(), String::new());
    assert_eq!(
        run_import(
            &CliContext::default(),
            &["--help".into()],
            &mut out,
            &mut err
        ),
        0
    );
    assert!(out.contains(
        "board-server import --repo <source> --board-id <16-hex> [--remote <name>] [--state <dir>]"
    ));
    assert!(err.is_empty());
    fn build(
        e: ImportCliEnvironment,
    ) -> Result<crate::application::project_board::use_cases::ImportBoardHistory, String> {
        assert_eq!(e.repo, PathBuf::from("/scratch/source"));
        assert_eq!(e.remote, "origin");
        Err("injected complete import graph".into())
    }
    let ctx = CliContext {
        board_history_import: Some(build),
        ..Default::default()
    };
    let mut command = vec!["import".into()];
    command.extend(args);
    out.clear();
    assert_eq!(
        super::super::cmd_board_server(&ctx, &command, &mut out, &mut err),
        1
    );
    assert!(err.contains("injected complete import graph"));
    assert!(out.is_empty());
}

/// #2741: export and import take the server's `--state` (absolute), and
/// carry the base directory the server's default state is under.
#[test]
fn export_and_import_take_the_servers_state_directory() {
    let (export, _) = parse(&args(), base()).unwrap();
    assert_eq!((export.base_dir.as_path(), export.state), (base(), None));
    let import: Vec<String> = [
        "--repo",
        "/scratch/source",
        "--board-id",
        "0123456789abcdef",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(parse_import(&import, base()).unwrap().state, None);
    for state in ["/scratch/state", "relative/state", ""] {
        let given = ["--state".to_owned(), state.to_owned()];
        let mut command = args();
        command.extend(given.clone());
        let mut imported = import.clone();
        imported.extend(given);
        let (export, import) = (parse(&command, base()), parse_import(&imported, base()));
        match state {
            "/scratch/state" => {
                assert_eq!(export.unwrap().0.state, Some(PathBuf::from(state)));
                assert_eq!(import.unwrap().state, Some(PathBuf::from(state)));
            }
            _ => assert!(export.is_err() && import.is_err(), "{state:?} refused"),
        }
    }
}
