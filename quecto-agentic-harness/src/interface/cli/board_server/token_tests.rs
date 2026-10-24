use super::*;

fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(String::from).collect()
}

#[test]
fn token_takes_add_revoke_and_list_with_a_socket() {
    let socket = PathBuf::from("/run/board.sock");
    for (text, action) in [
        (
            "add laptop --socket /run/board.sock",
            TokenAction::Add("laptop".into()),
        ),
        (
            "revoke laptop --socket /run/board.sock",
            TokenAction::Revoke("laptop".into()),
        ),
        ("list --socket /run/board.sock", TokenAction::List),
    ] {
        assert_eq!(
            parse(&args(text)),
            Ok(TokenOptions {
                action,
                socket: socket.clone()
            }),
            "{text}"
        );
    }
}

#[test]
fn anything_else_is_refused() {
    for refused in [
        "",
        "add",
        "add laptop",
        "add --socket /s",
        "add laptop phone --socket /s",
        "revoke --socket /s",
        "list",
        "list laptop --socket /s",
        "list --socket",
        "list --socket /s --socket /t",
        "remove laptop --socket /s",
    ] {
        let error = parse(&args(refused)).unwrap_err();
        assert!(
            error.starts_with("board-server: token"),
            "{refused}: {error}"
        );
    }
}
