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
        C::NotSetAside => &["path", "object"],
        C::NoPullRequestHost => &["reason"],
    }
}

/// Every [`ProjectBoardError`] variant, once: the pattern that matches it
/// and an error of it. [`variant`], [`every_error`] and [`VARIANTS`] are
/// all made from this one table, and `variant`'s match is exhaustive, so a
/// new variant fails to compile until it has its row here, and then it is
/// built and counted with the rest.
macro_rules! board_errors {
    ($($pattern:pat => $error:expr,)*) => {
        const VARIANTS: usize = [$(stringify!($pattern)),*].len();

        /// Which row of the table `error` is.
        fn variant(error: &ProjectBoardError) -> usize {
            use ProjectBoardError as E;
            let mut row = 0;
            $(
                if matches!(error, $pattern) {
                    return row;
                }
                row += 1;
            )*
            // Exhaustive: every variant has a row, so none falls through.
            match error {
                $($pattern => unreachable!("matched above at row {row}"),)*
            }
        }

        fn every_error() -> Vec<ProjectBoardError> {
            use ProjectBoardError as E;
            vec![$($error),*]
        }
    };
}

board_errors! {
    E::Store(BoardStoreError::NoBoard) => E::Store(BoardStoreError::NoBoard),
    E::Store(BoardStoreError::Clash(_)) => E::Store(BoardStoreError::Clash("a file".into())),
    E::Store(BoardStoreError::InvalidWrite(_)) => {
        E::Store(BoardStoreError::InvalidWrite("too big".into()))
    },
    E::Store(BoardStoreError::Unavailable(_)) => {
        E::Store(BoardStoreError::Unavailable("offline".into()))
    },
    E::Rule(_) => E::Rule(BoardRuleError::NotHolder),
    E::Invalid(_) => E::Invalid(SchemaError::new("title", "is empty")),
    E::NotFound(_) => E::NotFound(slug("a")),
    E::Exists(_) => E::Exists(slug("a")),
    E::Clock(BoardClockError::OutOfRange(_)) => {
        E::Clock(BoardClockError::OutOfRange("1969".into()))
    },
    E::Quarantined(_) => E::Quarantined(slug("a")),
    E::UnknownReference(_) => E::UnknownReference(slug("a")),
    E::ClosesCycle { .. } => E::ClosesCycle {
        id: slug("a"),
        by: slug("b"),
    },
    E::InvalidWriter(_) => E::InvalidWriter(SchemaError::new("title", "is empty")),
    E::PrStateUnknown(_) => E::PrStateUnknown(7),
    E::PullRequests(PullRequestStatesError::Unavailable(_)) => {
        E::PullRequests(PullRequestStatesError::Unavailable("no gh".into()))
    },
    E::TaskAhead { .. } => E::TaskAhead {
        id: slug("a"),
        updated: at("10:00:00"),
    },
    E::ClockOutOfRange(_) => E::ClockOutOfRange(at("10:00:00")),
    E::ClockAhead { .. } => E::ClockAhead {
        now: at("10:00:00"),
        reference: at("09:00:00"),
    },
    E::Contended { .. } => E::Contended { attempts: 5 },
    E::Refused(_) => E::Refused("protected".into()),
    E::NotSetAside { .. } => E::NotSetAside {
        path: "tasks/x.json".into(),
        object: "a".repeat(40),
    },
    E::NoPullRequestHost(_) => E::NoPullRequestHost(
        crate::application::project_board::dto::NoPullRequestHost::NotAPullRequestHost,
    ),
    E::Stale { .. } => E::Stale {
        id: slug("a"),
        expected: at("09:00:00"),
        updated: at("10:00:00"),
    },
}

#[test]
fn every_board_error_answers_its_codes_one_shape() {
    let errors = every_error();
    let covered: BTreeSet<usize> = errors.iter().map(variant).collect();
    assert_eq!(
        covered,
        (0..VARIANTS).collect(),
        "every_error builds every variant, each once"
    );
    assert_eq!(errors.len(), VARIANTS);
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
/// URL credentials and secret-shaped spans alike. A set-aside file's path
/// is the one exception: the store allowlists or quotes it, and a clear
/// must name it exactly as shown.
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
            path: "tasks/x.json".into(),
            id: None,
            reason: format!("api_key: {secret}"),
            object: "a".repeat(40),
        })}),
    ] {
        let text = answer.to_string();
        assert!(!text.contains(secret), "{text}");
    }
    let path = format!("tasks/{secret}.json");
    let shown = quarantined_json(&QuarantinedFile {
        path: path.clone(),
        id: None,
        reason: "a stray".into(),
        object: "a".repeat(40),
    });
    assert_eq!(shown["path"], json!(path), "a path is shown as it is");
}

/// A listing names what a clear needs: each file's object, and for a name
/// shown more than once every copy's object.
#[test]
fn a_listing_shows_each_files_object_and_its_names_copies() {
    let file = |path: &str, object: char| QuarantinedFile {
        path: path.into(),
        id: None,
        reason: "named more than once".into(),
        object: object.to_string().repeat(40),
    };
    let listed = quarantined_list(&[
        file("tasks/a.json", 'a'),
        file("tasks/a.json", 'b'),
        file("tasks/c.json", 'c'),
        // Two entries of one object (names no one can tell apart).
        file("tasks/c.json", 'c'),
    ]);
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    assert_eq!(listed[0]["object"], json!(a));
    assert_eq!(listed[0]["copies"], json!([a, b]));
    assert_eq!(listed[1]["copies"], json!([a, b]));
    assert_eq!(listed[2]["copies"], json!(["c".repeat(40)]));
    assert_eq!(listed[3]["copies"], json!(["c".repeat(40)]), "each once");
}

#[test]
fn a_path_shown_raw_holds_no_control_or_format_character() {
    assert!(plain_path("tasks/token=x.json"));
    assert!(plain_path("tasks/é ✓.json"));
    for hostile in [
        "tasks/a\u{0}.json",
        "tasks/a\u{1b}[31m.json",
        "tasks/a\u{202e}nosj.exe",
        "tasks/a\u{200b}.json",
        "tasks/a\u{e0041}.json",
        "tasks/a\u{feff}.json",
    ] {
        assert!(!plain_path(hostile), "{hostile:?}");
    }
}
