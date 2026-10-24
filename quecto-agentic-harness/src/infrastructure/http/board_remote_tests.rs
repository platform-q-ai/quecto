use super::*;

const TOKEN: &str = "c2VjcmV0LXRva2VuLWZvci10ZXN0cw";

#[test]
fn a_url_names_a_socket_or_an_allowlisted_address_with_its_token() {
    let unix = endpoint_of("unix:///run/board/board.sock", None).unwrap();
    assert_eq!(
        unix.endpoint(),
        &Endpoint::Unix("/run/board/board.sock".into())
    );
    let tcp = endpoint_of("http://100.64.1.2:7979", Some(TOKEN)).unwrap();
    assert_eq!(
        tcp.endpoint(),
        &Endpoint::Tcp("100.64.1.2:7979".parse().unwrap())
    );
    for refused in [
        "unix://relative.sock",
        "https://100.64.1.2:7979",
        "http://board.example:7979",
        "http://0.0.0.0:7979",
        "http://8.8.8.8:7979",
        "ftp://x",
    ] {
        assert!(endpoint_of(refused, Some(TOKEN)).is_err(), "{refused}");
    }
    let none = endpoint_of("http://127.0.0.1:7979", None).unwrap_err();
    // Rendered as a refusal is, redaction leaves it whole (round 1).
    let rendered = crate::domain::shared::redaction::redact_secrets(&none);
    assert_eq!(rendered, none);
    assert!(none.contains("QUECTO_BOARD_TOKEN"), "{none}");
    let bad = endpoint_of("http://127.0.0.1:7979", Some("bad token\r\n")).unwrap_err();
    assert!(!bad.contains("bad token"), "never shown: {bad}");
}

/// A 200 reply is its answer and facts; any other is the server's
/// refusal by its code, its words cut to shown characters.
#[test]
fn a_reply_is_an_answer_or_a_refusal_by_its_code() {
    let body = json!({"answer": {"ok": true, "op": "create"},
                      "facts": {"attempts": 1, "lost": 0, "head": "abc"}});
    let (answer, facts) = answered(200, body.to_string().as_bytes()).unwrap();
    let BoardOutcome::Served(ServedAnswer(answer)) = answer.outcome else {
        panic!("served")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&answer).unwrap()["op"],
        "create"
    );
    let facts = facts.unwrap();
    assert_eq!((facts.attempts, facts.lost), (Some(1), Some(0)));
    assert_eq!(facts.head, Some(BoardHead("abc".into())));
    let refused = answered(503, br#"{"error":"busy\u001b[2J now","code":"busy"}"#).unwrap_err();
    let said = refused.to_string();
    assert!(
        said.contains("(503 busy)") && !said.contains('\u{1b}'),
        "{said}"
    );
    let bare = answered(431, b"").unwrap_err().to_string();
    assert!(bare.contains("431 head_too_large"), "{bare}");
    assert!(
        answered(200, br#"{"answer": [1]}"#).is_err(),
        "an answer is an object"
    );
}

/// A start is refused before anything is sent: no server starts runs yet.
#[test]
fn a_start_is_refused_unsent() {
    let client = endpoint_of("unix:///nonexistent/board.sock", None).unwrap();
    let board = BoardId::of_hex("0123456789abcdef").unwrap();
    let remote = RemoteBoardOperations::new(vec![client], board, None);
    let value = json!({"op": "start", "task": "a", "plan_version": 1});
    let policy = crate::domain::project_board::entities::task_parts::EntryPolicy::Sectioned;
    let request = crate::interface::tools::project_board::request::parse(&value, policy).unwrap();
    let read = ReadRequest {
        request: &request,
        text: &value.to_string(),
        front_end: crate::domain::project_board::value_objects::telemetry::FrontEnd::Tool,
    };
    let refused = remote.execute(read, ListSummaries::Omitted, OpNoting::Unnoted);
    assert_eq!(
        refused.result,
        Err(ProjectBoardError::Refused(NO_START.into()))
    );
}

/// B7 round 3: a server that accepts and never replies fails a read once
/// its read reply time is past, and a write once its write reply time is
/// past, each sent once: a write that may have landed is never sent again
/// (only the in-flight write would answer, busy), and says so.
#[test]
fn a_hung_server_fails_a_read_and_a_write_once_each() {
    use super::super::board_service::client::CONNECT_TIMEOUT;
    use std::time::{Duration, Instant};
    assert_eq!(
        (READ_REPLY, WRITE_REPLY),
        (Duration::from_secs(30), Duration::from_secs(75))
    );
    assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(5));
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("hung.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&accepted);
    std::thread::spawn(move || {
        let mut held = vec![];
        for stream in listener.incoming() {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            held.push(stream);
        }
    });
    let client = endpoint_of(&format!("unix://{}", socket.display()), None).unwrap();
    let board = BoardId::of_hex("0123456789abcdef").unwrap();
    let ada = Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let mut remote = RemoteBoardOperations::new(vec![client], board, Some(ada));
    remote.replies = (Duration::from_millis(300), Duration::from_millis(800));
    let run = |value: Value| {
        let policy = crate::domain::project_board::entities::task_parts::EntryPolicy::Sectioned;
        let request =
            crate::interface::tools::project_board::request::parse(&value, policy).unwrap();
        let read = ReadRequest {
            request: &request,
            text: &value.to_string(),
            front_end: crate::domain::project_board::value_objects::telemetry::FrontEnd::Cli,
        };
        let began = Instant::now();
        (
            remote
                .execute(read, ListSummaries::Omitted, OpNoting::Unnoted)
                .result,
            began.elapsed(),
        )
    };
    let (listed, took) = run(json!({"op": "list"}));
    assert!(
        matches!(listed, Err(ProjectBoardError::Store(_))),
        "{listed:?}"
    );
    assert!(took < Duration::from_secs(2), "{took:?}");
    let count = || accepted.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(count(), 1, "a read with no reply is not sent again");
    let (made, took) = run(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    let Err(ProjectBoardError::Unanswered(why)) = made else {
        panic!("{made:?}")
    };
    assert!(took < Duration::from_secs(3), "{took:?}");
    assert_eq!(count(), 2, "a write with no reply is sent once");
    let said = ProjectBoardError::Unanswered(why).to_string();
    assert!(said.contains("the write may have landed"), "{said}");
}
