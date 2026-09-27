//! #2209: a failed request or read says its cause, its plain reason when
//! known and its redirect hop, bounded and without credentials.
use super::*;

/// One link of a cause chain.
#[derive(Debug)]
struct Layer {
    message: String,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}
impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl Error for Layer {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

/// A chain of `messages`, outermost first, ending at `root`.
fn chain(messages: &[&str], root: Box<dyn Error + Send + Sync>) -> Layer {
    let mut source = Some(root);
    for message in messages[1..].iter().rev() {
        source = Some(Box::new(Layer {
            message: (*message).to_owned(),
            source,
        }));
    }
    Layer {
        message: messages[0].to_owned(),
        source,
    }
}

fn io(kind: ErrorKind, message: &str) -> Box<dyn Error + Send + Sync> {
    Box::new(std::io::Error::new(kind, message.to_owned()))
}

fn url(text: &str) -> url::Url {
    url::Url::parse(text).unwrap()
}

/// The reqwest/hyper shape of a refused connection.
fn refused() -> Layer {
    chain(
        &[
            "error sending request",
            "client error (Connect)",
            "tcp connect error",
        ],
        io(
            ErrorKind::ConnectionRefused,
            "Connection refused (os error 111)",
        ),
    )
}

#[test]
fn the_cause_chain_is_joined_under_a_plain_reason() {
    let at = url("http://example.com:1/");
    assert_eq!(
        describe(&refused(), &at, &at),
        "connection refused: error sending request for url (http://example.com:1/): \
         client error (Connect): tcp connect error: Connection refused (os error 111)"
    );
}

#[test]
fn a_cause_already_said_is_not_repeated() {
    let at = url("http://example.com/");
    let error = chain(
        &["error sending request", "client error (Connect)"],
        Box::new(Layer {
            message: "client error (Connect)".into(),
            source: None,
        }),
    );
    assert_eq!(
        describe(&error, &at, &at),
        "error sending request for url (http://example.com/): client error (Connect)"
    );
}

#[test]
fn each_known_cause_has_its_plain_reason() {
    let at = url("https://example.com/");
    let cases: Vec<(Layer, &str)> = vec![
        (
            chain(
                &[
                    "error sending request",
                    "client error (Connect)",
                    "dns error",
                ],
                io(ErrorKind::Other, "failed to lookup address information"),
            ),
            "DNS lookup failed",
        ),
        (
            chain(
                &["error sending request", "client error (Connect)"],
                io(
                    ErrorKind::InvalidData,
                    "invalid peer certificate: UnknownIssuer",
                ),
            ),
            "TLS certificate not accepted",
        ),
        (
            chain(
                &["error sending request"],
                io(
                    ErrorKind::InvalidData,
                    "received fatal alert: HandshakeFailure",
                ),
            ),
            "TLS handshake failed",
        ),
        (
            chain(
                &["error sending request"],
                io(ErrorKind::InvalidData, "received corrupt message of type X"),
            ),
            "TLS handshake failed",
        ),
        (
            chain(&["error sending request"], io(ErrorKind::TimedOut, "t")),
            "timed out",
        ),
        (
            chain(
                &["error sending request"],
                io(ErrorKind::ConnectionReset, "r"),
            ),
            "connection reset by the server",
        ),
        (
            chain(
                &["error sending request"],
                io(ErrorKind::ConnectionAborted, "a"),
            ),
            "connection aborted",
        ),
        (
            chain(
                &["error sending request"],
                io(ErrorKind::HostUnreachable, "h"),
            ),
            "host unreachable",
        ),
        (
            chain(
                &["error sending request"],
                io(ErrorKind::NetworkUnreachable, "n"),
            ),
            "network unreachable",
        ),
    ];
    for (error, reason) in cases {
        let detail = describe(&error, &at, &at);
        assert!(detail.starts_with(&format!("{reason}: ")), "{detail}");
    }
}

/// Only the listed causes are named: any other reads as its chain alone.
#[test]
fn an_unknown_cause_has_no_plain_reason() {
    let at = url("http://example.com/");
    let error = chain(
        &["error sending request"],
        io(ErrorKind::PermissionDenied, "denied"),
    );
    assert_eq!(
        describe(&error, &at, &at),
        "error sending request for url (http://example.com/): denied"
    );
}

#[test]
fn a_failure_on_a_redirect_hop_names_the_hop() {
    let requested = url("https://httpbin.org/redirect-to?url=http://127.0.0.1:1/");
    let hop = url("http://127.0.0.1:1/");
    let detail = describe(&refused(), &requested, &hop);
    assert!(
        detail.starts_with(
            "connection refused on the redirect hop to http://127.0.0.1:1/: error sending request: "
        ),
        "{detail}"
    );
    let unknown = chain(&["error sending request"], io(ErrorKind::Other, "odd"));
    assert_eq!(
        describe(&unknown, &requested, &hop),
        "failed on the redirect hop to http://127.0.0.1:1/: error sending request: odd"
    );
}

/// Credentials carried to a hop and a fragment do not make it a hop, and
/// no URL is shown with its userinfo.
#[test]
fn credentials_are_never_shown_and_never_make_a_hop() {
    let requested = url("http://user:secret@example.com/a#part");
    let attempted = url("http://user:secret@example.com/a");
    let detail = describe(&refused(), &requested, &attempted);
    assert!(
        !detail.contains("secret") && !detail.contains("user@"),
        "{detail}"
    );
    assert!(!detail.contains("redirect hop"), "{detail}");
    assert!(detail.contains("(http://example.com/a#part)"), "{detail}");
    let hop = url("http://user:secret@other.example/");
    let detail = describe(&refused(), &requested, &hop);
    assert!(
        detail.contains("redirect hop to http://other.example/") && !detail.contains("secret"),
        "{detail}"
    );
}

#[test]
fn a_long_detail_is_cut_on_a_character_boundary() {
    let at = url("http://example.com/");
    for filler in ["x", "é", "€"] {
        let error = chain(
            &["error sending request"],
            io(ErrorKind::Other, &filler.repeat(600)),
        );
        let detail = describe(&error, &at, &at);
        assert!(detail.len() <= MAX_DETAIL_BYTES, "{}", detail.len());
        assert!(detail.len() > MAX_DETAIL_BYTES - 4, "{}", detail.len());
        assert!(detail.ends_with(CUT_MARK), "{detail}");
    }
    let short = describe(&refused(), &at, &at);
    assert!(!short.ends_with(CUT_MARK), "{short}");
    let exact = "e".repeat(MAX_DETAIL_BYTES);
    assert_eq!(bounded(exact.clone()), exact);
}

/// A cause with nothing to say adds no empty link to the chain.
#[test]
fn an_empty_cause_adds_nothing() {
    let at = url("http://example.com/");
    let error = chain(&["error sending request", ""], io(ErrorKind::Other, "odd"));
    assert_eq!(
        describe(&error, &at, &at),
        "error sending request for url (http://example.com/): odd"
    );
}
