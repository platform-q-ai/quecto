use super::*;
use serde_json::json;

fn decision_data() -> serde_json::Value {
    json!({
        "outcome": "decision",
        "code": "decision_required",
        "session": "cli:foreign",
        "sessionKey": "cli:foreign",
        "kind": "cross_folder",
        "homeVersion": "h1-0123456789abcdef",
        "executionPath": "/work/other",
        "detail": null,
        "actions": [
            {"action": "open_original", "available": false, "reason": "not yet (#2012)"},
            {"action": "fork_current", "available": false, "reason": null},
            {"action": "cancel", "available": true, "reason": null},
        ],
    })
}

#[test]
fn an_exact_selection_serializes_the_key_alone() {
    let json = serde_json::to_value(ResumeSelection::exact("chat-1")).unwrap();
    assert_eq!(json, json!({"session": "chat-1"}));
}

#[test]
fn an_action_selection_serializes_identity_action_and_version() {
    let json = serde_json::to_value(ResumeSelection {
        session: "chat-1".into(),
        action: Some(ResumeAction::ForkCurrent),
        expected_home_version: Some("h1-0123456789abcdef".into()),
    })
    .unwrap();
    assert_eq!(
        json,
        json!({
            "session": "chat-1",
            "action": "fork_current",
            "expectedHomeVersion": "h1-0123456789abcdef",
        })
    );
}

#[test]
fn every_action_has_its_stable_wire_name() {
    let names: Vec<_> = [
        ResumeAction::OpenOriginal,
        ResumeAction::ForkCurrent,
        ResumeAction::Locate,
        ResumeAction::Associate,
        ResumeAction::Cancel,
    ]
    .iter()
    .map(|action| serde_json::to_value(action).unwrap())
    .collect();
    assert_eq!(
        names,
        [
            "open_original",
            "fork_current",
            "locate",
            "associate",
            "cancel"
        ]
    );
}

#[test]
fn a_success_is_a_restore_unless_it_says_cancelled() {
    let resumed = json!({"outcome": "resumed", "session": "a"});
    let legacy = json!({"session": "a"});
    let cancelled = json!({"outcome": "cancelled", "session": "a"});
    assert_eq!(
        parse_resume_answer(true, Some(&resumed)),
        ResumeAnswer::Resumed
    );
    assert_eq!(
        parse_resume_answer(true, Some(&legacy)),
        ResumeAnswer::Resumed
    );
    assert_eq!(parse_resume_answer(true, None), ResumeAnswer::Resumed);
    assert_eq!(
        parse_resume_answer(true, Some(&cancelled)),
        ResumeAnswer::Cancelled
    );
}

#[test]
fn a_complete_decision_is_typed_with_availability_as_sent() {
    let ResumeAnswer::Decision(decision) = parse_resume_answer(false, Some(&decision_data()))
    else {
        panic!("decision expected");
    };
    assert_eq!(decision.kind, ResumeDecisionKind::CrossFolder);
    assert_eq!(decision.session_key, "cli:foreign");
    assert_eq!(decision.home_version, "h1-0123456789abcdef");
    assert_eq!(decision.execution_path.as_deref(), Some("/work/other"));
    let availability: Vec<_> = decision
        .actions
        .iter()
        .map(|offer| (offer.action, offer.available))
        .collect();
    assert_eq!(
        availability,
        [
            (ResumeAction::OpenOriginal, false),
            (ResumeAction::ForkCurrent, false),
            (ResumeAction::Cancel, true),
        ]
    );
    assert_eq!(
        decision.actions[0].reason.as_deref(),
        Some("not yet (#2012)")
    );
}

#[test]
fn every_kind_name_parses() {
    for (name, kind) in [
        ("cross_folder", ResumeDecisionKind::CrossFolder),
        ("home_missing", ResumeDecisionKind::HomeMissing),
        ("home_changed", ResumeDecisionKind::HomeChanged),
        ("home_unknown", ResumeDecisionKind::HomeUnknown),
        ("legacy_unscoped", ResumeDecisionKind::LegacyUnscoped),
    ] {
        let mut data = decision_data();
        data["kind"] = json!(name);
        let ResumeAnswer::Decision(decision) = parse_resume_answer(false, Some(&data)) else {
            panic!("{name}");
        };
        assert_eq!(decision.kind, kind);
    }
}

#[test]
fn an_unknown_kind_or_action_or_an_empty_offer_is_a_plain_refusal_never_a_guess() {
    let mut unknown_kind = decision_data();
    unknown_kind["kind"] = json!("teleport");
    let mut unknown_action = decision_data();
    unknown_action["actions"][0]["action"] = json!("restore_anyway");
    let mut no_actions = decision_data();
    no_actions["actions"] = json!([]);
    let mut success = decision_data();
    success["outcome"] = json!("decision");
    for data in [unknown_kind, unknown_action, no_actions] {
        assert_eq!(
            parse_resume_answer(false, Some(&data)),
            ResumeAnswer::Refused(None),
            "{data}"
        );
    }
    // A success is never a decision, whatever it carries.
    assert_eq!(
        parse_resume_answer(true, Some(&success)),
        ResumeAnswer::Resumed
    );
}

#[test]
fn a_refusal_carries_its_code_when_sent() {
    let stale = json!({"outcome": "refused", "code": "stale_home_version"});
    assert_eq!(
        parse_resume_answer(false, Some(&stale)),
        ResumeAnswer::Refused(Some("stale_home_version".into()))
    );
    assert_eq!(
        parse_resume_answer(false, None),
        ResumeAnswer::Refused(None)
    );
}

#[test]
fn hostile_metadata_is_made_safe_and_bounded() {
    let mut data = decision_data();
    data["session"] = json!("evil\u{1b}[2Jname");
    data["executionPath"] = json!(format!("/x\u{7}{}", "y".repeat(2000)));
    data["detail"] = json!("line\r\nbreak\u{1b}]0;title\u{7}");
    data["actions"][0]["reason"] = json!("why\u{1b}[31m");
    let ResumeAnswer::Decision(decision) = parse_resume_answer(false, Some(&data)) else {
        panic!("decision expected");
    };
    let texts = [
        decision.session.clone(),
        decision.execution_path.clone().unwrap(),
        decision.detail.clone().unwrap(),
        decision.actions[0].reason.clone().unwrap(),
    ];
    for text in texts {
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert!(text.chars().count() <= 512);
    }
}
