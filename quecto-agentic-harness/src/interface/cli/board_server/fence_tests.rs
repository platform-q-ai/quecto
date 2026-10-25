use super::*;
fn unexpected(_: FenceCliEnvironment) -> Result<FenceCliHandles, FenceError> {
    panic!("bad argv reached Git composition")
}
#[test]
fn fence_bad_options_never_reach_composition_or_make_state() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = CliContext {
        base_dir: Some(temp.path().to_owned()),
        board_fence: Some(unexpected),
        ..CliContext::default()
    };
    for text in [
        "",
        "--repo",
        "--board-id 0123456789abcdef --url https://example.test",
        "--repo /r --board-id BAD --url https://example.test",
        "--repo /r --board-id 0123456789abcdef",
        "--repo /r --repo /r --board-id 0123456789abcdef --url https://example.test",
        "--repo /r --board-id 0123456789abcdef --url ftp://example.test",
        "--repo /r --board-id 0123456789abcdef --url https://user:secret@example.test",
        "--repo /r --board-id 0123456789abcdef --url https://example.test --remote -unsafe",
        "--repo /r --board-id 0123456789abcdef --url https://example.test --state /bad",
    ] {
        let args: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
        let (mut stdout, mut stderr) = (String::new(), String::new());
        assert_eq!(run(&ctx, &args, &mut stdout, &mut stderr), 1, "{text}");
        let error: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(error["error"]["kind"], "invalid_arguments");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
#[test]
fn fence_parser_preserves_order_and_error_json_preserves_typed_moved() {
    let args = [
        "--repo",
        "/clone",
        "--board-id",
        "0123456789abcdef",
        "--url",
        "https://Primary.example.test/board",
        "--url",
        "http://standby.example.test:7979",
        "--remote",
        "mirror",
    ]
    .map(str::to_owned);
    let (environment, moved) = parse(&args).unwrap();
    assert_eq!(environment.remote, "mirror");
    assert_eq!(
        moved.urls(),
        [
            "https://Primary.example.test/board",
            "http://standby.example.test:7979"
        ]
    );
    let error = failure(&FenceError::Store(BoardStoreError::Moved(moved.clone())));
    assert_eq!(error["store"]["kind"], "moved");
    assert_eq!(error["store"]["board_id"], "0123456789abcdef");
    assert_eq!(error["store"]["urls"], json!(moved.urls()));
}
