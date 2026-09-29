//! A NULL `run.status` (#2270 review L2): a run row edited outside the
//! board, which Python reads as `None`. `None` is no status any decision
//! names, so each refuses as Python's comparisons do, and the refusals
//! write it `None`, as Python's f-strings do. A NULL member status is the
//! owner-decided `unknown_member_status_is_not_alive` divergence (#2295).
use serde_json::json;

use super::*;
use crate::domain::swarm::records::MemberState;

fn unset() -> RunRecord {
    RunRecord {
        status: None,
        coordinator: Some("parent".into()),
        deadline: 100.0,
        member_limit: 2,
        outcome: Some("succeeded".into()),
        outcome_reason: Some("done".into()),
    }
}

fn live(id: &str) -> MemberRecord {
    MemberRecord {
        id: id.into(),
        status: Some(MemberState::LIVE),
        reservation: Some("r".into()),
    }
}

fn message<T: std::fmt::Debug>(result: Result<T, BoardError>) -> String {
    result.unwrap_err().0
}

/// `describe` returns `run['status']`, `None`, whatever outcome the row
/// holds: only a `'paused'` status names one.
#[test]
fn describe_writes_a_null_status_as_none() {
    assert_eq!(describe(&unset()), "None");
}

/// `authorize`: `None != 'running'` refuses active work; any other access
/// passes the run check.
#[test]
fn authorize_refuses_active_work_on_a_null_status() {
    let parent = live("parent");
    let active = Access {
        active: true,
        ..Access::default()
    };
    assert_eq!(
        message(authorize(Some(&unset()), "parent", Some(&parent), active)),
        "run is None; no new work permitted"
    );
    let coordinating = Access {
        coordinator: true,
        ..Access::default()
    };
    for access in [Access::default(), coordinating] {
        assert_eq!(
            authorize(Some(&unset()), "parent", Some(&parent), access),
            Ok(())
        );
    }
}

/// `expired`: `None == 'running'` is false, so a past deadline neither
/// expires the run nor exhausts its budget.
#[test]
fn a_null_status_never_expires() {
    assert!(!expired(&unset(), 1_000.0));
    assert_eq!(require_budget(&unset(), 1_000.0), Ok(()));
}

/// `admission`: `None not in ('setup', 'running')`.
#[test]
fn admission_refuses_a_null_status() {
    assert_eq!(
        message(admission(&unset(), None, &json!("r"), 0, 0.0)),
        "run is None; no new admission"
    );
}

/// A NULL member status (the `unknown_member_status_is_not_alive`
/// divergence, owner decision in #2295): Python's `None == 'dead'` is
/// false, so it counts as alive there; Rust accepts only `live` or
/// `reserved`, so the member may read but not mutate, and holding the same
/// reservation is not an idempotent admission retry.
#[test]
fn a_null_member_status_is_not_alive() {
    let unknown = MemberRecord {
        status: None,
        ..live("parent")
    };
    let running = RunRecord {
        status: Some(RunState::RUNNING),
        ..unset()
    };
    assert_eq!(
        message(authorize(
            Some(&running),
            "parent",
            Some(&unknown),
            Access::default()
        )),
        "invoking member is unknown or death confirmed"
    );
    let reading = Access {
        read_only: true,
        ..Access::default()
    };
    assert_eq!(
        authorize(Some(&running), "parent", Some(&unknown), reading),
        Ok(())
    );
    assert_eq!(
        message(admission(&running, Some(&unknown), &json!("r"), 0, 0.0)),
        "member identity already used; choose a stable new identity"
    );
}
