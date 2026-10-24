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
