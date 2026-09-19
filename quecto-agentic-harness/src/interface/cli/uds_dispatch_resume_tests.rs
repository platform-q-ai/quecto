use super::*;
use crate::application::sessions::conversation_ledger::LedgerAdvance;
use crate::application::sessions::dto::{
    ResumeActionCapabilities, ResumeActionOffer, ResumeTarget, SavedSessionResumed,
};
use crate::domain::resume_decision::{HomeVersion, ResumeAction, ResumeDecisionKind};
use crate::domain::session_home::SessionHomeScope;
use std::path::PathBuf;

fn json(event: &AgentEvent) -> serde_json::Value {
    serde_json::from_str(&event.to_json_line()).unwrap()
}

fn decision(kind: ResumeDecisionKind) -> ResumeDecision {
    let capabilities = ResumeActionCapabilities::cancel_only();
    ResumeDecision {
        target: ResumeTarget::parse("cli:foreign").unwrap(),
        kind,
        home_version: HomeVersion::of(&SessionHomeScope::LegacyUnscoped),
        execution_dir: Some(PathBuf::from("/work/else\u{1b}[2Jwhere")),
        detail: Some("bell\u{7}".into()),
        offers: kind
            .offered_actions()
            .iter()
            .map(|action| ResumeActionOffer {
                action: *action,
                availability: capabilities.availability(*action),
            })
            .collect(),
    }
}

#[test]
fn a_restore_is_acknowledged_with_the_name_key_and_length() {
    let target = ResumeTarget::parse("saved").unwrap();
    let outcome = ResumeOutcome::Resumed(SavedSessionResumed {
        name: target.name,
        identity: target.identity,
        message_count: 4,
        ledger: LedgerAdvance {
            epoch: 2,
            rev: 0,
            changed: true,
        },
    });
    let event = json(&outcome_event(Some("r1"), "resume_session", &outcome));
    assert_eq!(event["success"], true);
    assert_eq!(event["id"], "r1");
    assert_eq!(
        event["data"],
        serde_json::json!({
            "outcome": "resumed",
            "session": "saved",
            "sessionKey": "cli:saved",
            "messageCount": 4,
        })
    );
}

#[test]
fn a_cancel_is_acknowledged_as_cancelled_and_nothing_else() {
    let outcome = ResumeOutcome::Cancelled {
        name: "saved".into(),
    };
    let event = json(&outcome_event(None, "resume_session", &outcome));
    assert_eq!(event["success"], true);
    assert_eq!(
        event["data"],
        serde_json::json!({"outcome": "cancelled", "session": "saved"})
    );
}

#[test]
fn a_decision_is_a_failure_carrying_the_typed_safe_decision() {
    let refusal =
        ResumeSavedSessionError::Decision(Box::new(decision(ResumeDecisionKind::CrossFolder)));
    let event = json(&refusal_event(Some("r2"), "resume_session", &refusal));
    assert_eq!(event["success"], false);
    assert_eq!(event["command"], "resume_session");
    let data = &event["data"];
    assert_eq!(data["outcome"], "decision");
    assert_eq!(data["code"], "decision_required");
    assert_eq!(data["kind"], "cross_folder");
    assert_eq!(data["session"], "cli:foreign");
    assert_eq!(data["sessionKey"], "cli:foreign");
    assert!(data["homeVersion"].as_str().unwrap().starts_with("h1-"));
    assert_eq!(data["executionPath"], "/work/else\u{fffd}[2Jwhere");
    assert_eq!(data["detail"], "bell\u{fffd}");
    let actions = data["actions"].as_array().unwrap();
    let names: Vec<_> = actions
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["open_original", "fork_current", "cancel"]);
    assert_eq!(actions[0]["available"], false);
    assert!(actions[0]["reason"].as_str().unwrap().contains("#2012"));
    assert_eq!(actions[2]["available"], true);
    assert_eq!(actions[2]["reason"], serde_json::Value::Null);
    assert!(
        event["error"]
            .as_str()
            .unwrap()
            .starts_with("session resume unavailable:")
    );
}

#[test]
fn a_decision_without_a_path_or_detail_says_null() {
    let mut legacy = decision(ResumeDecisionKind::LegacyUnscoped);
    legacy.execution_dir = None;
    legacy.detail = None;
    let refusal = ResumeSavedSessionError::Decision(Box::new(legacy));
    let data = json(&refusal_event(None, "resume_session", &refusal))["data"].clone();
    assert_eq!(data["kind"], "legacy_unscoped");
    assert_eq!(data["executionPath"], serde_json::Value::Null);
    assert_eq!(data["detail"], serde_json::Value::Null);
}

#[test]
fn an_unavailable_action_names_the_action_asked_for_and_the_reason() {
    let refusal = ResumeSavedSessionError::ActionUnavailable {
        action: ResumeAction::ForkCurrent,
        reason: "not yet\u{1b}".into(),
    };
    let event = json(&refusal_event(None, "resume_session", &refusal));
    assert_eq!(
        event["data"],
        serde_json::json!({
            "outcome": "refused",
            "code": "action_unavailable",
            "action": "fork_current",
            "reason": "not yet\u{fffd}",
        })
    );
    let elsewhere = ResumeSavedSessionError::ActionExecutedElsewhere(ResumeAction::Locate);
    let event = json(&refusal_event(None, "resume_session", &elsewhere));
    assert_eq!(
        event["data"],
        serde_json::json!({
            "outcome": "refused",
            "code": "action_executed_elsewhere",
            "action": "locate",
        })
    );
}

#[test]
fn every_other_refusal_carries_its_code_and_a_safe_message() {
    let refusal = ResumeSavedSessionError::NotFound("gone\u{1b}[31m".into());
    let event = json(&refusal_event(Some("r3"), "resume_session", &refusal));
    assert_eq!(event["success"], false);
    assert_eq!(
        event["data"],
        serde_json::json!({"outcome": "refused", "code": "not_found"})
    );
    assert_eq!(event["error"], "session not found: gone\u{fffd}[31m");
    let stale = ResumeSavedSessionError::StaleHomeVersion;
    let event = json(&refusal_event(None, "resume_session", &stale));
    assert_eq!(event["data"]["code"], "stale_home_version");
}
