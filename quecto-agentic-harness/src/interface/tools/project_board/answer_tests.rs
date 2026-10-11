use std::collections::BTreeSet;

use super::*;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

/// The keys each code's error object carries, every time.
fn keys(code: ErrorCode) -> &'static [&'static str] {
    use ErrorCode as C;
    match code {
        C::InvalidRequest | C::InvalidTask | C::InvalidWriter => &["field", "problem"],
        C::NoCommitter
        | C::Clash
        | C::InvalidWrite
        | C::StoreUnavailable
        | C::PullRequestsUnavailable
        | C::Refused => &["reason"],
        C::NoBoard => &[],
        C::Rule => &["rule"],
        C::NotFound | C::Exists | C::SetAside | C::UnknownReference => &["task"],
        C::ClockUnreadable => &["read"],
        C::ClosesCycle => &["task", "by"],
        C::PrStateUnknown => &["pr"],
        C::TaskAhead => &["task", "updated"],
        C::ClockOutOfRange => &["reference"],
        C::ClockAhead => &["now", "reference"],
        C::Contended => &["attempts"],
        C::Stale => &["task", "expected", "updated"],
        C::NotSetAside => &["path"],
    }
}

/// Which variant `error` is: an exhaustive match, so a new variant fails
/// to compile until it is added to [`every_error`].
fn variant(error: &ProjectBoardError) -> &'static str {
    use ProjectBoardError as E;
    match error {
        E::Store(BoardStoreError::NoBoard) => "no board",
        E::Store(BoardStoreError::Clash(_)) => "clash",
        E::Store(BoardStoreError::InvalidWrite(_)) => "invalid write",
        E::Store(BoardStoreError::Unavailable(_)) => "unavailable",
        E::Rule(_) => "rule",
        E::Invalid(_) => "invalid",
        E::NotFound(_) => "not found",
        E::Exists(_) => "exists",
        E::Clock(BoardClockError::OutOfRange(_)) => "clock",
        E::Quarantined(_) => "quarantined",
        E::UnknownReference(_) => "unknown reference",
        E::ClosesCycle { .. } => "closes cycle",
        E::InvalidWriter(_) => "invalid writer",
        E::PrStateUnknown(_) => "pr state unknown",
        E::PullRequests(PullRequestStatesError::Unavailable(_)) => "pull requests",
        E::TaskAhead { .. } => "task ahead",
        E::ClockOutOfRange(_) => "clock out of range",
        E::ClockAhead { .. } => "clock ahead",
        E::Contended { .. } => "contended",
        E::Refused(_) => "refused",
        E::NotSetAside(_) => "not set aside",
        E::Stale { .. } => "stale",
    }
}

fn every_error() -> Vec<ProjectBoardError> {
    use ProjectBoardError as E;
    let schema = || SchemaError::new("title", "is empty");
    vec![
        E::Store(BoardStoreError::NoBoard),
        E::Store(BoardStoreError::Clash("a file".into())),
        E::Store(BoardStoreError::InvalidWrite("too big".into())),
        E::Store(BoardStoreError::Unavailable("offline".into())),
        E::Rule(BoardRuleError::NotHolder),
        E::Invalid(schema()),
        E::NotFound(slug("a")),
        E::Exists(slug("a")),
        E::Clock(BoardClockError::OutOfRange("1969".into())),
        E::Quarantined(slug("a")),
        E::UnknownReference(slug("a")),
        E::ClosesCycle {
            id: slug("a"),
            by: slug("b"),
        },
        E::InvalidWriter(schema()),
        E::PrStateUnknown(7),
        E::PullRequests(PullRequestStatesError::Unavailable("no gh".into())),
        E::TaskAhead {
            id: slug("a"),
            updated: at("10:00:00"),
        },
        E::ClockOutOfRange(at("10:00:00")),
        E::ClockAhead {
            now: at("10:00:00"),
            reference: at("09:00:00"),
        },
        E::Contended { attempts: 5 },
        E::Refused("protected".into()),
        E::NotSetAside("tasks/x.json".into()),
        E::Stale {
            id: slug("a"),
            expected: at("09:00:00"),
            updated: at("10:00:00"),
        },
    ]
}

#[test]
fn every_board_error_answers_its_codes_one_shape() {
    let errors = every_error();
    let covered: BTreeSet<&str> = errors.iter().map(variant).collect();
    assert_eq!(covered.len(), errors.len(), "each variant listed once");
    for error in &errors {
        let answer = board_refused("show", error);
        assert_eq!(answer["ok"], json!(false));
        let object = answer["error"].as_object().unwrap();
        let code = object["code"].as_str().unwrap();
        let (expected, _) = error_fields(error);
        assert_eq!(code, expected.as_str(), "{error:?}");
        let mut want: BTreeSet<&str> = keys(expected).iter().copied().collect();
        want.extend(["code", "message"]);
        let got: BTreeSet<&str> = object.keys().map(String::as_str).collect();
        assert_eq!(got, want, "{error:?}");
    }
}

#[test]
fn a_front_ends_own_refusals_have_their_codes_shapes() {
    for (refusal, code) in [
        (FrontEndRefusal::NoCommitter, ErrorCode::NoCommitter),
        (
            FrontEndRefusal::StoreUnavailable,
            ErrorCode::StoreUnavailable,
        ),
    ] {
        let answer = reason_refused(Some("list"), refusal, "why");
        assert_eq!(answer["error"]["code"], json!(code.as_str()));
        let got: BTreeSet<&str> = answer["error"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut want: BTreeSet<&str> = keys(code).iter().copied().collect();
        want.extend(["code", "message"]);
        assert_eq!(got, want);
    }
}

/// Every text an answer carries from outside the harness is redacted:
/// URL credentials and secret-shaped spans alike.
#[test]
fn text_from_outside_is_redacted_wherever_it_is_shown() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123";
    let leaky = SchemaError::new(format!("plan {secret}"), format!("token: {secret}"));
    for answer in [
        board_refused("create", &ProjectBoardError::Invalid(leaky.clone())),
        board_refused("create", &ProjectBoardError::InvalidWriter(leaky.clone())),
        board_refused(
            "create",
            &ProjectBoardError::Rule(BoardRuleError::Invalid(leaky.clone())),
        ),
        board_refused(
            "complete",
            &ProjectBoardError::Refused(format!("https://u:{secret}@github.com/x")),
        ),
        reason_refused(
            None,
            FrontEndRefusal::StoreUnavailable,
            &format!("password={secret}"),
        ),
        json!({"quarantined": quarantined_json(&QuarantinedFile {
            path: format!("tasks/{secret}.json"),
            id: None,
            reason: format!("api_key: {secret}"),
        })}),
    ] {
        let text = answer.to_string();
        assert!(!text.contains(secret), "{text}");
    }
}
