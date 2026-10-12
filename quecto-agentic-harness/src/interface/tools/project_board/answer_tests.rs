use std::collections::BTreeSet;

use super::*;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    DiscussionLimit, PlanState, TaskStatus,
};

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
        C::SetAsideFile => &["path", "object"],
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
    E::SetAsideFile { .. } => E::SetAsideFile {
        path: "tasks/?.json".into(),
        object: "a".repeat(40),
    },
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

/// A store never shows a path with a control or format character; an
/// answer that would show one stops rather than pass it on.
#[test]
#[should_panic(expected = "shows every path it sets aside sanitised")]
fn an_unsanitised_path_is_never_shown() {
    let _ = quarantined_json(&QuarantinedFile {
        path: "tasks/a\u{202e}nosj.exe".into(),
        id: None,
        reason: "a stray".into(),
        object: "a".repeat(40),
    });
}

/// Every board rule, once: the pattern that matches it, a rule of it, its
/// code and the keys its answer carries. The test's match is exhaustive,
/// so a new rule fails to compile until it has its row here.
macro_rules! board_rules {
    ($($pattern:pat => $rule:expr, $code:literal, [$($key:literal),*];)*) => {
        fn every_rule() -> Vec<(BoardRuleError, &'static str, Vec<&'static str>)> {
            use BoardRuleError as R;
            vec![$(($rule, $code, vec!["code" $(, $key)*])),*]
        }

        fn rule_row(rule: &BoardRuleError) -> &'static str {
            use BoardRuleError as R;
            match rule {
                $($pattern => $code,)*
            }
        }
    };
}

board_rules! {
    R::NotAllowed { .. } => R::NotAllowed { from: TaskStatus::Draft, to: TaskStatus::Done }, "not_allowed", ["from", "to"];
    R::NotHolder => R::NotHolder, "not_holder", [];
    R::NoClaim => R::NoClaim, "no_claim", [];
    R::ClaimLive => R::ClaimLive, "claim_live", [];
    R::ClaimExpired => R::ClaimExpired, "claim_expired", [];
    R::RunOpen(_) => R::RunOpen(1), "run_open", ["run"];
    R::RunEnded(_) => R::RunEnded(1), "run_ended", ["run"];
    R::PlanNotDraft(_) => R::PlanNotDraft(PlanState::Accepted), "plan_not_draft", ["state"];
    R::NotThisTasks => R::NotThisTasks, "not_this_tasks", [];
    R::NotThisChange => R::NotThisChange, "not_this_change", [];
    R::TaskFinished(_) => R::TaskFinished(TaskStatus::Done), "task_finished", ["status"];
    R::UnknownPlanVersion(_) => R::UnknownPlanVersion(3), "unknown_plan_version", ["version"];
    R::PlanChanged { .. } => R::PlanChanged { version: 2, updated: at("10:00:00") }, "plan_changed", ["version", "updated"];
    R::PlansInconsistent(_) => R::PlansInconsistent("two accepted".into()), "plans_inconsistent", ["reason"];
    R::RunsInconsistent(_) => R::RunsInconsistent("two going".into()), "runs_inconsistent", ["reason"];
    R::PlanNotAccepted(_) => R::PlanNotAccepted(2), "plan_not_accepted", ["version"];
    R::NotTakeable(_) => R::NotTakeable(TaskStatus::Ready), "not_takeable", ["status"];
    R::AlreadyHolder => R::AlreadyHolder, "already_holder", [];
    R::NoPrs => R::NoPrs, "no_prs", [];
    R::PrsUnmerged(_) => R::PrsUnmerged(vec![7]), "prs_unmerged", ["prs"];
    R::PrsLinked(_) => R::PrsLinked(vec![7]), "prs_linked", ["prs"];
    R::PrNotLinked(_) => R::PrNotLinked(7), "pr_not_linked", ["pr"];
    R::PrAlreadyLinked(_) => R::PrAlreadyLinked(7), "pr_already_linked", ["pr"];
    R::NotDraft(_) => R::NotDraft(TaskStatus::Ready), "not_draft", ["status"];
    R::NoEdit => R::NoEdit, "no_edit", [];
    R::Superseded { .. } => R::Superseded { by: vec![Slug::parse("next").unwrap()] }, "superseded", ["by"];
    R::IssueShared { .. } => R::IssueShared { issue: 7, tasks: [Slug::parse("a").unwrap(), Slug::parse("b").unwrap()] }, "issue_shared", ["issue", "tasks"];
    R::IssueIsPr { .. } => R::IssueIsPr { issue: 7, task: Slug::parse("a").unwrap(), pr_holder: Slug::parse("b").unwrap() }, "issue_is_pr", ["issue", "task", "pr_holder"];
    R::OwnPrReferenced { .. } => R::OwnPrReferenced { task: Slug::parse("a").unwrap(), pr: 7, change: 1 }, "own_pr_referenced", ["task", "pr", "change"];
    R::ChangeAlreadyLinked { .. } => R::ChangeAlreadyLinked { task: Slug::parse("board-store").unwrap(), change: Some(1) }, "change_already_linked", ["task", "change"];
    R::NoSuchChange(_) => R::NoSuchChange(4), "no_such_change", ["change"];
    R::ChangeAhead { .. } => R::ChangeAhead { change: 1, updated: at("10:00:00") }, "change_ahead", ["change", "updated"];
    R::DiscussionClosed(_) => R::DiscussionClosed(Slug::parse("chat").unwrap()), "discussion_closed", ["discussion"];
    R::DiscussionLinked { .. } => R::DiscussionLinked { discussion: Slug::parse("chat").unwrap(), task: Slug::parse("board-store").unwrap() }, "discussion_linked", ["discussion", "task"];
    R::DiscussionAhead { .. } => R::DiscussionAhead { discussion: Slug::parse("chat").unwrap(), updated: at("10:00:00") }, "discussion_ahead", ["discussion", "updated"];
    R::MessageAhead { .. } => R::MessageAhead { discussion: Slug::parse("chat").unwrap(), at: at("10:00:00") }, "message_ahead", ["discussion", "at"];
    R::DiscussionFull { .. } => R::DiscussionFull { discussion: Slug::parse("chat").unwrap(), limit: DiscussionLimit::Lines }, "discussion_full", ["discussion", "limit"];
    R::ReviewAhead { .. } => R::ReviewAhead { change: 1, updated: at("10:00:00") }, "review_ahead", ["change", "updated"];
    R::ChangesLinked(_) => R::ChangesLinked(vec![1]), "changes_linked", ["changes"];
    R::PrUnread(_) => R::PrUnread(7), "pr_unread", ["pr"];
    R::PrNotOpen(_) => R::PrNotOpen(7), "pr_not_open", ["pr"];
    R::PrHeadMismatch(_) => R::PrHeadMismatch(7), "pr_head_mismatch", ["pr"];
    R::ChangeUnlinked(_) => R::ChangeUnlinked(1), "change_unlinked", ["change"];
    R::ChangeMerged(_) => R::ChangeMerged(1), "change_merged", ["change"];
    R::NoChanges => R::NoChanges, "no_changes", [];
    R::ChangesUnmerged(_) => R::ChangesUnmerged(vec![1]), "changes_unmerged", ["changes"];
    R::ChangesInconsistent(_) => R::ChangesInconsistent("number 1 twice".into()), "changes_inconsistent", ["reason"];
    R::PlanAhead { .. } => R::PlanAhead { version: 2, updated: at("10:00:00") }, "plan_ahead", ["version", "updated"];
    R::Invalid(_) => R::Invalid(SchemaError::new("title", "is empty")), "invalid", ["field", "problem"];
}

#[test]
fn every_rule_answers_its_code_and_its_own_keys() {
    let rules = every_rule();
    let codes: BTreeSet<&str> = rules.iter().map(|(rule, ..)| rule_row(rule)).collect();
    assert_eq!(codes.len(), rules.len(), "every rule, each once");
    for (rule, code, keys) in rules {
        let shown = rule_json(&rule);
        assert_eq!(shown["code"], json!(code), "{rule:?}");
        let got: BTreeSet<&str> = shown
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let want: BTreeSet<&str> = keys.into_iter().collect();
        assert_eq!(got, want, "{rule:?}");
    }
    // A run is named by its number.
    assert_eq!(rule_json(&BoardRuleError::RunOpen(3))["run"], json!(3));
    // A pull request linked the old way names no change.
    let old_way = BoardRuleError::ChangeAlreadyLinked {
        task: Slug::parse("board-store").unwrap(),
        change: None,
    };
    assert_eq!(rule_json(&old_way)["change"], json!(null));
    assert_eq!(
        old_way.to_string(),
        "task board-store already links that pull request"
    );
}

/// A set-aside file of no known task is named by its path as the listing
/// shows it, raw, with its object, so a clear can name it exactly: a name
/// that looks like a secret is no secret in a path.
#[test]
fn a_set_aside_file_is_named_by_its_raw_path_and_object() {
    let shown = error_json(&ProjectBoardError::SetAsideFile {
        path: "tasks/token=x.json".into(),
        object: "a".repeat(40),
    });
    assert_eq!(shown["path"], json!("tasks/token=x.json"));
    assert_eq!(shown["object"], json!("a".repeat(40)));
}
