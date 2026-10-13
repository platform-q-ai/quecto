//! A conditional claim admits only the record its condition names, checked
//! with the claim itself; a refusal claims nothing, changes nothing and
//! writes nothing to the journal.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::super::*;

fn record(env_ref: &str, status: EnvironmentStatus, created_by: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: format!("env-{env_ref}"),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["kill.sh".to_string()],
        retained_cleanup_argv: vec!["cleanup.sh".to_string()],
        retained_inspect_argv: vec![],
        members: vec![],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: created_by.to_string(),
        created_at: None,
    }
}

/// A durable registry whose journal records every status it is asked to
/// write.
fn journalled() -> (EnvironmentRegistry, Arc<Mutex<Vec<String>>>) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&writes);
    let journal = EnvironmentJournal {
        allocate_ref: Arc::new(|floor| Ok(floor.max(1))),
        release_ref: Arc::new(|_| {}),
        recorded: Arc::new(
            move |record: &EnvironmentRecord, _: Option<&EnvironmentStatus>| {
                recorded.lock().unwrap().push(format!(
                    "{} {}",
                    record.environment_ref,
                    record.status_label()
                ));
                JournalWrite::Written
            },
        ),
        forgotten: Arc::new(|_: &EnvironmentRecord| {}),
        reload: Arc::new(|| Ok(Vec::new())),
    };
    (
        EnvironmentRegistry::with_journal(journal, "cli:test"),
        writes,
    )
}

const POCKET: &str = "cli:pocket-7";

fn key(text: &str) -> CreatorKey {
    CreatorKey::try_from(text.to_string()).unwrap()
}

#[test]
fn a_creator_key_is_nonempty_printable_ascii_without_spaces() {
    for text in [
        POCKET,
        "telegram:-42",
        "chat-1760000000-7",
        "a",
        &"k".repeat(256),
    ] {
        assert_eq!(key(text).as_str(), text);
    }
    for text in [
        "",
        " ",
        "cli:pocket-7 ",
        " cli:pocket-7",
        "cli pocket",
        "tab\tkey",
        "lf\nkey",
        "é",
        &"k".repeat(257),
    ] {
        assert_eq!(
            CreatorKey::try_from(text.to_string()),
            Err(InvalidCreatorKey),
            "{text:?}"
        );
    }
}

/// A key can never be empty, so no condition admits a session-less record.
#[test]
fn a_session_less_record_is_never_admitted_by_a_creator_condition() {
    let none = record("C1", EnvironmentStatus::Running, "");
    for text in [POCKET, "-", "cli:"] {
        assert!(
            !ClaimCondition::CreatedBy(key(text)).admits(&none),
            "{text}"
        );
    }
    assert!(ClaimCondition::Any.admits(&none));
}

/// Exact bytes: no case folding, no trimming.
#[test]
fn a_creator_differing_in_case_or_whitespace_is_not_admitted() {
    let condition = ClaimCondition::CreatedBy(key(POCKET));
    for created_by in [
        "CLI:POCKET-7",
        "cli:pocket-7 ",
        " cli:pocket-7",
        "cli:pocket-7\n",
    ] {
        let other = record("C1", EnvironmentStatus::Running, created_by);
        assert!(!condition.admits(&other), "{created_by:?}");
    }
    assert_eq!(condition.judge_creator("C4", POCKET), Ok(()));
    assert_eq!(
        condition.judge_creator("C4", "cli:other"),
        Err(CreatorMismatch {
            environment_ref: "C4".into(),
            expected: POCKET.into(),
            created_by: "cli:other".into(),
        })
    );
}

#[test]
fn any_admits_every_record_and_created_by_only_an_exact_key() {
    let ours = record("C1", EnvironmentStatus::Running, POCKET);
    let theirs = record("C1", EnvironmentStatus::Running, "cli:pocket-70");
    let none = record("C1", EnvironmentStatus::Running, "");
    assert!(ClaimCondition::Any.admits(&ours));
    assert!(ClaimCondition::Any.admits(&none));
    let condition = ClaimCondition::CreatedBy(key(POCKET));
    assert!(condition.admits(&ours));
    assert!(!condition.admits(&theirs), "a prefix is not a match");
    assert!(!condition.admits(&none));
    assert!(!ClaimCondition::CreatedBy(key("cli:pocket")).admits(&ours));
    assert_eq!(
        condition.judge(&theirs),
        Err(CreatorMismatch {
            environment_ref: "C1".into(),
            expected: POCKET.into(),
            created_by: "cli:pocket-70".into(),
        })
    );
}

#[test]
fn a_kill_claim_is_granted_when_the_creator_matches() {
    let (registry, writes) = journalled();
    registry.commit(record("C1", EnvironmentStatus::Running, POCKET));
    writes.lock().unwrap().clear();
    let claim = registry
        .begin_kill_if("C1", &ClaimCondition::CreatedBy(key(POCKET)))
        .unwrap();
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Killing
    );
    assert_eq!(*writes.lock().unwrap(), ["C1 killing"]);
    registry.complete_kill(claim);
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn a_kill_claim_on_another_creators_record_is_refused_untouched() {
    let (registry, writes) = journalled();
    registry.commit(record("C1", EnvironmentStatus::Running, "cli:other"));
    writes.lock().unwrap().clear();
    let refusal = registry
        .begin_kill_if("C1", &ClaimCondition::CreatedBy(key(POCKET)))
        .unwrap_err();
    assert_eq!(
        refusal,
        ClaimRefusal::CreatedByOther(CreatorMismatch {
            environment_ref: "C1".into(),
            expected: POCKET.into(),
            created_by: "cli:other".into(),
        })
    );
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Running
    );
    assert!(writes.lock().unwrap().is_empty(), "nothing was journalled");
    // The ordinary claim is still free to take.
    assert!(registry.begin_kill("C1").is_ok());
}

#[test]
fn a_conditional_kill_claim_keeps_the_ordinary_refusals() {
    let (registry, _) = journalled();
    registry.commit(record("C1", EnvironmentStatus::Stopped, POCKET));
    let condition = ClaimCondition::CreatedBy(key(POCKET));
    assert_eq!(
        registry.begin_kill_if("C1", &condition).unwrap_err(),
        ClaimRefusal::Lookup(EnvironmentLookupError::Stopped("C1".into()))
    );
    assert_eq!(
        registry.begin_kill_if("C9", &condition).unwrap_err(),
        ClaimRefusal::Lookup(EnvironmentLookupError::Unknown("C9".into()))
    );
}

#[test]
fn a_removal_claim_is_granted_only_when_the_creator_matches() {
    let (registry, writes) = journalled();
    registry.commit(record("C1", EnvironmentStatus::Stopped, "cli:other"));
    registry.commit(record("C2", EnvironmentStatus::Stopped, POCKET));
    writes.lock().unwrap().clear();
    let condition = ClaimCondition::CreatedBy(key(POCKET));
    assert_eq!(
        registry.begin_removal_if("C1", &condition).unwrap_err(),
        ClaimRefusal::CreatedByOther(CreatorMismatch {
            environment_ref: "C1".into(),
            expected: POCKET.into(),
            created_by: "cli:other".into(),
        })
    );
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Stopped
    );
    assert!(writes.lock().unwrap().is_empty(), "nothing was journalled");
    let claim = registry.begin_removal_if("C2", &condition).unwrap();
    assert_eq!(claim.claimed_from(), &EnvironmentStatus::Stopped);
    assert_eq!(
        registry.get("C2").unwrap().status,
        EnvironmentStatus::Killing
    );
    assert_eq!(
        registry.begin_removal_if("C9", &condition).unwrap_err(),
        ClaimRefusal::Lookup(EnvironmentLookupError::Unknown("C9".into()))
    );
}
