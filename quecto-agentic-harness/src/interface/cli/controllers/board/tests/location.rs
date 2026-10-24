//! #2705 B1: `--board-id` and `--remote-url`, read from the command line
//! and passed to composition.
use super::*;

/// A board built only when told the location the next test names.
fn located_board(environment: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    use crate::domain::project_board::value_objects::telemetry::BoardId;
    assert_eq!(
        environment.known,
        KnownLocation {
            board_id: BoardId::of_hex("0123456789abcdef"),
            remote_url: Some("https://github.com/acme/widgets.git".into()),
        },
        "the location the command line names reaches composition"
    );
    memory_board(environment)
}

/// A board built only when told no location.
fn unlocated_board(
    environment: &ProjectBoardEnvironment,
) -> Result<ProjectBoardHandles, BuildError> {
    assert_eq!(environment.known, KnownLocation::default());
    memory_board(environment)
}

/// #2705 B1: `--board-id` and `--remote-url` name where the board is, so
/// composition need not ask git; without them it asks, as before.
#[test]
fn the_board_id_and_remote_url_reach_composition() {
    let (code, answer, _) = cli(
        Some(located_board),
        &[
            "list",
            "--board-id",
            "0123456789abcdef",
            "--remote-url",
            "https://github.com/acme/widgets.git",
        ],
    );
    assert_eq!((code, &answer["ok"]), (0, &json!(true)), "{answer}");
    let (code, answer, _) = cli(Some(unlocated_board), &["list"]);
    assert_eq!((code, &answer["ok"]), (0, &json!(true)), "{answer}");
}

#[test]
fn a_board_id_or_remote_url_the_command_line_cannot_take_is_refused() {
    let id = "must be 16 lower-case hex digits";
    let url = "must be at most 4096 bytes of shown characters and spaces";
    let twice_id = [
        "list",
        "--board-id",
        "0123456789abcdef",
        "--board-id",
        "0123456789abcdef",
    ];
    for (line, field, problem) in [
        (
            &["list", "--board-id", "0123456789ABCDEF"][..],
            "board-id",
            id,
        ),
        (
            &["list", "--board-id", "0123456789abcde"][..],
            "board-id",
            id,
        ),
        (&["list", "--board-id", "../../etc"][..], "board-id", id),
        (&["list", "--board-id", ""][..], "board-id", "needs a value"),
        (&twice_id[..], "board-id", "is given twice"),
        (
            &["list", "--remote-url", "https://x\n/y"][..],
            "remote-url",
            url,
        ),
        (
            &["list", "--remote-url", ""][..],
            "remote-url",
            "needs a value",
        ),
        (
            &["list", "--remote-url", "a", "--remote-url", "a"][..],
            "remote-url",
            "is given twice",
        ),
    ] {
        let (code, answer, _) = cli(Some(never_built), line);
        assert_eq!(code, 1, "{line:?}");
        assert_eq!(
            answer["error"]["code"],
            json!("invalid_request"),
            "{line:?}"
        );
        assert_eq!(answer["error"]["field"], json!(field), "{line:?}");
        assert_eq!(answer["error"]["problem"], json!(problem), "{line:?}");
    }
    let long = format!("https://example.com/{}", "a".repeat(4096));
    let (code, answer, _) = cli(Some(never_built), &["list", "--remote-url", &long]);
    assert_eq!((code, &answer["error"]["field"]), (1, &json!("remote-url")));
    // A local path with a space is a URL git takes.
    let line = read_line(&args(&["list", "--remote-url", "/srv/my repos/r.git"])).unwrap();
    assert_eq!(
        line.known.remote_url.as_deref(),
        Some("/srv/my repos/r.git")
    );
}
