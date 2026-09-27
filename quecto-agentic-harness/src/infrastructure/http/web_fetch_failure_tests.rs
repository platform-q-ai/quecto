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
        describe(&refused(), Attempt::Requested(&at)),
        "connection refused: error sending request: client error (Connect): \
         tcp connect error: Connection refused (os error 111), for url (http://example.com:1/)"
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
        describe(&error, Attempt::Requested(&at)),
        "error sending request: client error (Connect), for url (http://example.com/)"
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
        let detail = describe(&error, Attempt::Requested(&at));
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
        describe(&error, Attempt::Requested(&at)),
        "error sending request: denied, for url (http://example.com/)"
    );
}

#[test]
fn a_failure_on_a_redirect_hop_names_the_hop() {
    let hop = url("http://127.0.0.1:1/");
    let detail = describe(&refused(), Attempt::Hop(&hop));
    assert!(
        detail.starts_with(
            "connection refused on the redirect hop to http://127.0.0.1:1/: error sending request: "
        ),
        "{detail}"
    );
    let unknown = chain(&["error sending request"], io(ErrorKind::Other, "odd"));
    assert_eq!(
        describe(&unknown, Attempt::Hop(&hop)),
        "failed on the redirect hop to http://127.0.0.1:1/: error sending request: odd"
    );
}

/// No URL is shown with its userinfo, its query or its fragment, on the
/// URL asked for or on a hop (#2248 review M3).
#[test]
fn credentials_and_queries_are_never_shown() {
    let requested = url("http://user:secret@example.com/a?access_token=SECRET123#part");
    let detail = describe(&refused(), Attempt::Requested(&requested));
    assert!(
        detail.ends_with(", for url (http://example.com/a?…)"),
        "{detail}"
    );
    let hop = url("http://user:secret@other.example/cb?code=SECRET123&state=x#frag");
    let detail = describe(&refused(), Attempt::Hop(&hop));
    assert!(
        detail.starts_with("connection refused on the redirect hop to http://other.example/cb?…: "),
        "{detail}"
    );
    for leaked in ["secret", "SECRET123", "user@", "part", "frag", "state"] {
        for detail in [
            describe(&refused(), Attempt::Requested(&requested)),
            describe(&refused(), Attempt::Hop(&hop)),
        ] {
            assert!(!detail.contains(leaked), "{leaked}: {detail}");
        }
    }
}

/// Scheme, host, port and path are shown as they are; an empty query or
/// fragment adds no marker.
#[test]
fn a_url_keeps_its_scheme_host_port_and_path() {
    assert_eq!(
        shown(&url("https://u:p@example.com:8443/a/b?x=1#f")),
        "https://example.com:8443/a/b?…"
    );
    assert_eq!(
        shown(&url("http://example.com/a?#")),
        "http://example.com/a"
    );
    assert_eq!(shown(&url("http://[::1]:1/")), "http://[::1]:1/");
}

/// #2248 review M2: a URL is shown at most [`MAX_URL_BYTES`] long, cut on a
/// character boundary, so the cause after it always survives the bound.
#[test]
fn a_long_url_never_pushes_the_cause_out() {
    for filler in ["x", "é", "€"] {
        let long = url(&format!(
            "http://example.com/{}?q={}",
            filler.repeat(300),
            "y".repeat(600)
        ));
        let shown_url = shown(&long);
        assert!(shown_url.len() <= MAX_URL_BYTES, "{}", shown_url.len());
        assert!(shown_url.len() > MAX_URL_BYTES - 4, "{}", shown_url.len());
        assert!(shown_url.ends_with(CUT_MARK), "{shown_url}");
        let unknown = chain(
            &["error sending request"],
            io(ErrorKind::Other, "the real cause"),
        );
        for detail in [
            describe(&unknown, Attempt::Requested(&long)),
            describe(&unknown, Attempt::Hop(&long)),
            describe(&refused(), Attempt::Hop(&long)),
        ] {
            assert!(detail.len() <= MAX_DETAIL_BYTES, "{}", detail.len());
            assert!(
                detail.contains("the real cause")
                    || detail.contains("Connection refused (os error 111)"),
                "{detail}"
            );
        }
    }
    let query = url(&format!("http://example.com/?q={}", "y".repeat(600)));
    let unknown = chain(
        &["error sending request"],
        io(ErrorKind::Other, "the real cause"),
    );
    assert_eq!(
        describe(&unknown, Attempt::Requested(&query)),
        "error sending request: the real cause, for url (http://example.com/?…)"
    );
}

/// #2248 review L2: a hint added after the detail stays within the bound,
/// the detail cut to make room for it.
#[test]
fn a_hint_stays_within_the_bound() {
    let hint = "(a hint)";
    let long = "x".repeat(MAX_DETAIL_BYTES);
    let hinted = with_hint(&long, hint);
    assert!(hinted.len() <= MAX_DETAIL_BYTES, "{}", hinted.len());
    assert!(hinted.ends_with(&format!("{CUT_MARK} {hint}")), "{hinted}");
    assert_eq!(with_hint("short", hint), "short (a hint)");
    let exact = "e".repeat(MAX_DETAIL_BYTES - hint.len() - 1);
    assert_eq!(with_hint(&exact, hint), format!("{exact} {hint}"));
}

#[test]
fn a_long_detail_is_cut_on_a_character_boundary() {
    let at = url("http://example.com/");
    for filler in ["x", "é", "€"] {
        let error = chain(
            &["error sending request"],
            io(ErrorKind::Other, &filler.repeat(600)),
        );
        let detail = describe(&error, Attempt::Requested(&at));
        assert!(detail.len() <= MAX_DETAIL_BYTES, "{}", detail.len());
        assert!(detail.len() > MAX_DETAIL_BYTES - 4, "{}", detail.len());
        assert!(detail.ends_with(CUT_MARK), "{detail}");
    }
    let short = describe(&refused(), Attempt::Requested(&at));
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
        describe(&error, Attempt::Requested(&at)),
        "error sending request: odd, for url (http://example.com/)"
    );
}
