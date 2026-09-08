//! Shared assertions used by real loopback observations and sensitivity controls.
//! Each control accepts a valid observation, then rejects independent bad values.
use quecto::domain::inference_admission::Feedback;

pub fn sends(actual: usize, expected: usize) {
    assert_eq!(actual, expected, "physical HTTP send count");
}
pub fn endpoint(actual: &str, expected: &str) {
    assert_eq!(actual, expected, "request endpoint");
}
pub fn header(actual: Option<&reqwest::header::HeaderValue>) {
    assert_eq!(
        actual.and_then(|value| value.to_str().ok()),
        Some("configured"),
        "configured builder header"
    );
}
pub fn grants(actual: usize, expected: usize) {
    assert_eq!(actual, expected, "admission acquisitions");
}
pub fn status(error: &str, expected: u16) {
    assert!(
        error.contains(&expected.to_string()),
        "missing original HTTP status: {error}"
    );
}
pub fn finishes(actual: &[Feedback], expected: &[Feedback]) {
    assert_eq!(actual, expected, "permit completion feedback");
}
pub fn body(original: &[u8], replayed: &[u8]) {
    assert_eq!(original, replayed, "307/308 preserves JSON POST");
}
pub fn failure<T: std::fmt::Debug, E: std::fmt::Display>(result: Result<T, E>) -> String {
    result
        .expect_err("HTTP failure must not succeed")
        .to_string()
}
pub fn terminal(error: Option<String>, done: bool) -> String {
    assert!(!done, "redirect must not succeed");
    error.expect("terminal error")
}

fn rejects(check: impl FnOnce() + std::panic::UnwindSafe) {
    assert!(
        std::panic::catch_unwind(check).is_err(),
        "counterexample escaped shared assertion"
    );
}

#[test]
fn sends_control_and_counterexamples() {
    for expected in [1, 2, 3] {
        sends(expected, expected);
        rejects(|| sends(expected - 1, expected));
        rejects(|| sends(expected + 1, expected));
    }
}
#[test]
fn endpoint_control_and_counterexamples() {
    for expected in [
        "/chat/completions",
        "/responses",
        "/codex/responses",
        "/v1/messages",
        "/redirect-target",
    ] {
        endpoint(expected, expected);
        rejects(|| endpoint("/wrong", expected));
    }
}
#[test]
fn header_control_and_counterexamples() {
    header(Some(&"configured".parse().unwrap()));
    rejects(|| header(None));
    rejects(|| header(Some(&"default-client".parse().unwrap())));
}
#[test]
fn grants_control_and_counterexamples() {
    grants(0, 0);
    grants(1, 1);
    rejects(|| grants(1, 0));
    rejects(|| grants(0, 1));
    rejects(|| grants(2, 1));
}
#[test]
fn status_control_and_counterexamples() {
    for expected in [307, 308, 503] {
        status(&format!("HTTP {expected} from provider"), expected);
        rejects(|| status("HTTP 400 from provider", expected));
        rejects(|| status("request failed", expected));
    }
}
#[test]
fn finishes_control_and_counterexamples() {
    finishes(&[], &[]);
    finishes(&[Feedback::Failure], &[Feedback::Failure]);
    rejects(|| finishes(&[Feedback::Failure], &[]));
    rejects(|| finishes(&[], &[Feedback::Failure]));
    rejects(|| finishes(&[Feedback::Success], &[Feedback::Failure]));
    rejects(|| {
        finishes(
            &[Feedback::Failure, Feedback::Failure],
            &[Feedback::Failure],
        )
    });
}
#[test]
fn body_control_and_counterexamples() {
    body(br#"{"model":"fixture"}"#, br#"{"model":"fixture"}"#);
    rejects(|| body(br#"{"model":"fixture"}"#, b""));
    rejects(|| body(br#"{"model":"fixture"}"#, br#"{"model":"changed"}"#));
}
#[test]
fn failure_control_and_counterexamples() {
    failure::<(), _>(Err("HTTP 503"));
    rejects(|| {
        failure::<_, &str>(Ok(()));
    });
}
#[test]
fn terminal_control_and_counterexamples() {
    terminal(Some("HTTP 307".into()), false);
    rejects(|| {
        terminal(None, false);
    });
    rejects(|| {
        terminal(Some("HTTP 307".into()), true);
    });
    rejects(|| {
        terminal(None, true);
    });
}
