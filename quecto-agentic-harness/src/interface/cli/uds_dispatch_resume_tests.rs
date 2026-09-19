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
    let target = ResumeTarget::parse("cli:foreign").unwrap();
    ResumeDecision {
        home_version: HomeVersion::of(&target.identity, &SessionHomeScope::LegacyUnscoped),
        target,
        kind,
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
    let reason = actions[0]["reason"].as_str().unwrap();
    assert!(reason.contains("start quecto in that folder"), "{reason}");
    assert!(
        !reason.contains('#'),
        "no tracker number for people: {reason}"
    );
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

/// The pre-transaction refusals are typed too (R1-H2, R1-H10): the busy edge
/// and an action that names no version.
#[test]
fn the_busy_and_version_required_refusals_are_typed() {
    let event = json(&refusal_event(
        Some("r4"),
        "resume_session",
        &ResumeSavedSessionError::Busy,
    ));
    assert_eq!(
        event["data"],
        serde_json::json!({"outcome": "refused", "code": "busy"})
    );
    assert_eq!(
        event["error"],
        "cannot resume a session while agent is running"
    );
    let refusal = ResumeSavedSessionError::HomeVersionRequired(ResumeAction::Associate);
    let event = json(&refusal_event(None, "resume_session", &refusal));
    assert_eq!(
        event["data"],
        serde_json::json!({
            "outcome": "refused",
            "code": "home_version_required",
            "action": "associate",
        })
    );
}

/// Every character that invisibly reorders, hides or splits text.
fn invisible_characters() -> impl Iterator<Item = char> {
    ['\u{ad}', '\u{34f}', '\u{61c}', '\u{feff}']
        .into_iter()
        .chain('\u{180b}'..='\u{180f}')
        .chain('\u{200b}'..='\u{200f}')
        .chain('\u{2028}'..='\u{202e}')
        .chain('\u{2060}'..='\u{206f}')
        .chain('\u{fff9}'..='\u{fffb}')
        .chain('\u{e0000}'..='\u{e007f}')
}

/// Invisible reordering and zero-width characters never reach a client
/// (R1-T6, R2-H2): every Bidi_Control character (U+061C included), zero-width
/// characters and marks, line and paragraph separators, the soft hyphen, the
/// invisible operators, interlinear annotations, tag characters, the BOM.
#[test]
fn a_decisions_texts_carry_no_bidi_or_zero_width_characters() {
    let mut hostile = decision(ResumeDecisionKind::CrossFolder);
    let path = "/srv/\u{202e}gpj.exe\u{202c}/a\u{200b}b\u{2066}c\u{2069}\u{feff}\u{200f}";
    hostile.execution_dir = Some(std::path::PathBuf::from(path));
    hostile.detail = Some(invisible_characters().collect());
    let refusal = ResumeSavedSessionError::Decision(Box::new(hostile));
    let event = json(&refusal_event(None, "resume_session", &refusal));
    let text = event.to_string();
    for ch in invisible_characters() {
        assert!(!text.contains(ch), "U+{:04X} in {text}", ch as u32);
    }
    assert_eq!(
        event["data"]["executionPath"],
        "/srv/\u{fffd}gpj.exe\u{fffd}/a\u{fffd}b\u{fffd}c\u{fffd}\u{fffd}\u{fffd}"
    );
}

/// The review's live reproduction (R2-H2), and what stays: an emoji's
/// variation selector is presentation, not concealment.
#[test]
fn the_arabic_letter_mark_separators_and_soft_hyphen_are_replaced() {
    let mut hostile = decision(ResumeDecisionKind::HomeMissing);
    let path = "/var/tmp/x\u{061c}\u{2028}\u{2060}\u{00ad}\u{202e}y\u{2764}\u{fe0f}";
    hostile.execution_dir = Some(std::path::PathBuf::from(path));
    let refusal = ResumeSavedSessionError::Decision(Box::new(hostile));
    let event = json(&refusal_event(None, "resume_session", &refusal));
    assert_eq!(
        event["data"]["executionPath"],
        "/var/tmp/x\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}y\u{2764}\u{fe0f}"
    );
}

/// R3-H2: one folder, one spelling — the decision's `executionPath` is the
/// list row's, byte for byte, for a folder no lossy text could carry.
#[test]
fn a_decision_spells_its_folder_exactly_as_the_listed_row_does() {
    use crate::application::sessions::dto::ListedSession;
    use crate::domain::session_home::{AssociationProvenance, SessionHome, WorkspaceGroup};
    use std::os::unix::ffi::OsStrExt;
    let dir = PathBuf::from(std::ffi::OsStr::from_bytes(b"/w/caf\xe9/a\\b\x1b[2J"));
    let mut shown = decision(ResumeDecisionKind::CrossFolder);
    shown.execution_dir = Some(dir.clone());
    let target = ResumeTarget::parse("cli:foreign").unwrap();
    let row = ListedSession {
        summary: crate::domain::session::SessionSummary {
            key: "cli:foreign".into(),
            identity: target.identity,
            title: "t".into(),
            message_count: 1,
            updated_unix_secs: None,
        },
        home: SessionHomeScope::Scoped(SessionHome {
            execution_dir: dir.clone(),
            group: WorkspaceGroup::Folder { directory: dir },
            provenance: AssociationProvenance::SavedHere,
        }),
        resume_eligible: false,
    };
    let row = super::super::super::uds_dispatch_query::listed_row_json(&row);
    let decided = decision_json(&shown, "cross_folder");
    assert_eq!(decided["executionPath"], "/w/caf\\xE9/a\\\\b\u{fffd}[2J");
    assert_eq!(decided["executionPath"], row["executionPath"]);
}
