//! Membership decisions over Python-compared values (#2271 round-1
//! review M1, L3): admission compares the reservation as Python's `==`
//! does, and one status allowlist decides a member's liveness.
use serde_json::json;

use super::*;
use crate::domain::swarm::records::{MemberState, RunState};

fn running() -> RunRecord {
    RunRecord {
        status: Some(RunState::RUNNING),
        coordinator: Some("parent".into()),
        deadline: 100.0,
        member_limit: 2,
        outcome: None,
        outcome_reason: None,
    }
}

fn member(id: &str, status: MemberState, reservation: &str) -> MemberRecord {
    MemberRecord {
        id: id.into(),
        status: Some(status),
        reservation: Some(reservation.into()),
    }
}

fn message<T: std::fmt::Debug>(result: Result<T, BoardError>) -> String {
    result.expect_err("the policy refuses").to_string()
}

/// The reservation is compared with Python's `==` (#2271 round-1 review
/// M1): a stored `'5'` is not the argument `5`, and a stored `'1'` is not
/// `True`, so neither is a retry.
#[test]
fn admission_compares_the_reservation_as_python_does() {
    let numeric = member("worker", MemberState::RESERVED, "5");
    assert_eq!(
        admission(&running(), Some(&numeric), &json!("5"), 1, 50.0),
        Ok(false)
    );
    for given in [json!(5), json!(5.0), json!(["5"])] {
        assert_eq!(
            message(admission(&running(), Some(&numeric), &given, 1, 50.0)),
            "member identity already used; choose a stable new identity",
            "{given}"
        );
    }
    let one = member("worker", MemberState::LIVE, "1");
    assert_eq!(
        message(admission(&running(), Some(&one), &json!(true), 1, 50.0)),
        "member identity already used; choose a stable new identity"
    );
}

/// One allowlist decides a member's liveness (#2271 round-1 review L3):
/// `live` and `reserved` only.
#[test]
fn only_live_and_reserved_statuses_are_alive() {
    for (status, alive) in [
        (Some("live"), true),
        (Some("reserved"), true),
        (Some("dead"), false),
        (Some("LIVE"), false),
        (Some(""), false),
        (Some("zombie"), false),
        (None, false),
    ] {
        assert_eq!(status_is_alive(status), alive, "{status:?}");
    }
}
