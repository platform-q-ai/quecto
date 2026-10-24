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

/// Review round 2: a name is judged by an allowlist of its shape.
#[test]
fn a_name_is_shaped_by_an_allowlist() {
    let longest = "a".repeat(64);
    for allowed in ["laptop", "pixel-8", "8", "a-b-c", longest.as_str()] {
        assert!(name_shaped(allowed), "{allowed}");
        assert!(
            parse(&args(&format!("add {allowed} --socket /s"))).is_ok(),
            "{allowed}"
        );
    }
    let too_long = "a".repeat(65);
    for refused in [
        "",
        "-x",
        "--socket",
        "Laptop",
        "my_laptop",
        "a.b",
        "a b",
        "é",
        too_long.as_str(),
    ] {
        assert!(!name_shaped(refused), "{refused}");
        let command = vec![
            "revoke".to_string(),
            refused.to_string(),
            "--socket".into(),
            "/s".into(),
        ];
        assert!(parse(&command).is_err(), "{refused}");
    }
}
