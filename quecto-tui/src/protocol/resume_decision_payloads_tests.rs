use super::*;
use serde_json::json;

fn refusal_data() -> serde_json::Value {
    json!({
        "outcome": "refused", "code": "belongs_elsewhere", "kind": "cross_folder",
        "session": "foreign", "sessionKey": "cli:foreign",
        "executionPath": "/work/other", "detail": null,
        "command": "cd '/work/other' && quecto-tui", "resume": "/resume cli:foreign",
    })
}

#[test]
fn a_selection_serializes_the_key_and_the_listed_version_and_never_an_action() {
    let exact = serde_json::to_value(ResumeSelection::exact("cli:a")).unwrap();
    assert_eq!(exact, json!({"session": "cli:a"}));
    let listed = ResumeSelection {
        session: "cli:a".into(),
        expected_home_version: Some("h1-0123456789abcdef".into()),
    };
    assert_eq!(
        serde_json::to_value(listed).unwrap(),
        json!({"session": "cli:a", "expectedHomeVersion": "h1-0123456789abcdef"})
    );
}

fn ack(name: &str, key: Option<&str>) -> ResumeAnswer {
    ResumeAnswer::Resumed(ResumeSessionAck {
        name: name.into(),
        session_key: key.map(str::to_string),
    })
}

#[test]
fn only_a_success_that_says_resumed_or_names_no_outcome_is_a_restore() {
    let resumed = json!({"outcome": "resumed", "session": "a", "sessionKey": "cli:a"});
    let legacy = json!({"session": "a"});
    assert_eq!(
        parse_resume_answer(true, Some(&resumed)),
        ack("a", Some("cli:a"))
    );
    assert_eq!(parse_resume_answer(true, Some(&legacy)), ack("a", None));
    assert_eq!(parse_resume_answer(true, None), ack("session", None));
}

/// all change nothing — however much they look like an acknowledgement.
#[test]
fn every_other_success_is_unrecognized_and_never_a_restore() {
    for outcome in [
        "opened_elsewhere",
        "forked",
        "decision",
        "refused",
        "Resumed",
        "",
        " resumed",
    ] {
        let data =
            json!({"outcome": outcome, "session": "cli:foreign", "sessionKey": "cli:foreign"});
        assert_eq!(
            parse_resume_answer(true, Some(&data)),
            ResumeAnswer::Unrecognized(Some(outcome.to_string())),
            "{outcome:?}"
        );
    }
    for garbage in [
        json!({"outcome": 7, "sessionKey": "cli:foreign"}),
        json!({"outcome": "resumed", "sessionKey": ["cli:foreign"]}),
        json!({"session": 7}),
        json!("resumed"),
        json!([1, 2]),
        json!(null),
    ] {
        assert_eq!(
            parse_resume_answer(true, Some(&garbage)),
            ResumeAnswer::Unrecognized(None),
            "{garbage}"
        );
    }
    // The same garbage on a failure is a plain refusal.
    assert_eq!(
        parse_resume_answer(false, Some(&json!({"outcome": 7}))),
        ResumeAnswer::Refused(None)
    );
}

#[test]
fn a_restore_without_a_readable_name_keeps_the_toast_visible() {
    assert_eq!(
        parse_resume_answer(true, Some(&json!({}))),
        ack("session", None)
    );
    assert_eq!(parse_resume_answer(true, None), ack("session", None));
}

#[test]
fn a_typed_refusal_carries_code_kind_folder_command_and_resume_step() {
    let ResumeAnswer::Elsewhere(refusal) = parse_resume_answer(false, Some(&refusal_data())) else {
        panic!("a typed refusal");
    };
    assert_eq!(refusal.code, ResumeRefusalCode::BelongsElsewhere);
    assert_eq!(refusal.kind, "cross_folder");
    assert_eq!(refusal.session_key.as_deref(), Some("cli:foreign"));
    assert_eq!(refusal.execution_path.as_deref(), Some("/work/other"));
    assert_eq!(
        refusal.command.as_deref(),
        Some("cd '/work/other' && quecto-tui")
    );
    assert_eq!(refusal.resume.as_deref(), Some("/resume cli:foreign"));
}

#[test]
fn every_home_refusal_code_parses() {
    for (wire, code) in [
        ("belongs_elsewhere", ResumeRefusalCode::BelongsElsewhere),
        ("home_missing", ResumeRefusalCode::HomeMissing),
        ("home_changed", ResumeRefusalCode::HomeChanged),
        ("home_unknown", ResumeRefusalCode::HomeUnknown),
        ("no_home_recorded", ResumeRefusalCode::NoHomeRecorded),
    ] {
        let mut data = refusal_data();
        data["code"] = json!(wire);
        let ResumeAnswer::Elsewhere(refusal) = parse_resume_answer(false, Some(&data)) else {
            panic!("{wire}");
        };
        assert_eq!(refusal.code, code);
    }
}

/// A refusal that is not about the session's folder — or that this TUI cannot
/// read — is a plain refusal (a toast) that keeps its code when it had one:
/// never a panel and never a guess.
#[test]
fn any_other_refusal_is_plain_and_keeps_its_code() {
    for (data, code) in [
        (
            json!({"outcome": "refused", "code": "not_found"}),
            Some("not_found"),
        ),
        (
            json!({"outcome": "refused", "code": "stale_home_version"}),
            Some("stale_home_version"),
        ),
        (
            json!({"outcome": "refused", "code": "legacy_action_unsupported"}),
            Some("legacy_action_unsupported"),
        ),
        // A home code without its kind, a kind without a code, a code this
        // TUI does not know: not enough for the notice.
        (
            json!({"outcome": "refused", "code": "belongs_elsewhere"}),
            Some("belongs_elsewhere"),
        ),
        (json!({"outcome": "refused", "kind": "cross_folder"}), None),
        (
            json!({"outcome": "refused", "code": "opened_elsewhere", "kind": "cross_folder"}),
            Some("opened_elsewhere"),
        ),
    ] {
        assert_eq!(
            parse_resume_answer(false, Some(&data)),
            ResumeAnswer::Refused(code.map(str::to_string)),
            "{data}"
        );
    }
    for unreadable in [
        json!({"outcome": "refused", "code": 7, "kind": "cross_folder"}),
        json!({"outcome": "resumed", "code": "belongs_elsewhere", "kind": "cross_folder"}),
        json!("not an object"),
    ] {
        assert_eq!(
            parse_resume_answer(false, Some(&unreadable)),
            ResumeAnswer::Refused(None),
            "{unreadable}"
        );
    }
    assert_eq!(
        parse_resume_answer(false, None),
        ResumeAnswer::Refused(None)
    );
}

/// A pre-#2045 harness answers `outcome: "decision"` with `actions`: the same
/// notice, read from the kind; it sent no command and no resume step.
#[test]
fn an_older_harnesss_decision_reads_as_the_same_refusal() {
    let data = json!({
        "outcome": "decision", "code": "decision_required", "kind": "home_missing",
        "session": "cli:foreign", "sessionKey": "cli:foreign",
        "homeVersion": "h1-0123456789abcdef", "executionPath": "/work/gone",
        "detail": "No such file or directory",
        "actions": [{"action": "locate", "available": false, "reason": "…"},
                    {"action": "cancel", "available": true, "reason": null}],
    });
    let ResumeAnswer::Elsewhere(refusal) = parse_resume_answer(false, Some(&data)) else {
        panic!("typed");
    };
    assert_eq!(refusal.code, ResumeRefusalCode::HomeMissing);
    assert_eq!((refusal.command, refusal.resume), (None, None));
    let unknown = json!({"outcome": "decision", "code": "decision_required", "kind": "new_kind"});
    assert_eq!(
        parse_resume_answer(false, Some(&unknown)),
        ResumeAnswer::Refused(Some("decision_required".to_string()))
    );
}
