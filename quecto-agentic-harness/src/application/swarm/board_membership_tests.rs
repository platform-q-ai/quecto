use serde_json::{Value, json};

use super::{alive, holds_reservation, launched_elsewhere, live, same_process, unlaunched};
use crate::application::swarm::board_test_support::stored_member;
use crate::application::swarm::dto::{LaunchIdentity, MemberRow};

fn row(reservation: Value, status: &str, pid: Value, started: Value) -> MemberRow {
    stored_member(
        "m",
        reservation,
        status,
        [pid, started, json!(null), json!(null)],
    )
}

fn launch(pid: i64, started: &str) -> LaunchIdentity {
    loose(json!(pid), json!(started))
}

fn loose(pid: Value, started: Value) -> LaunchIdentity {
    LaunchIdentity { pid, started }
}

#[test]
fn statuses_are_matched_affirmatively() {
    for (status, is_alive, is_live) in [
        ("live", true, true),
        ("reserved", true, false),
        ("dead", false, false),
        ("LIVE", false, false),
        ("", false, false),
    ] {
        let member = row(json!("r"), status, json!(null), json!(null));
        assert_eq!(
            (alive(&member), live(&member)),
            (is_alive, is_live),
            "{status}"
        );
    }
    let mut null_status = row(json!("r"), "live", json!(null), json!(null));
    null_status.columns[2].1 = Value::Null;
    assert!(!alive(&null_status) && !live(&null_status));
}

#[test]
fn only_a_reserved_row_without_a_pid_is_unlaunched() {
    assert!(unlaunched(&row(
        json!("r"),
        "reserved",
        json!(null),
        json!(null)
    )));
    assert!(!unlaunched(&row(
        json!("r"),
        "reserved",
        json!(0),
        json!(null)
    )));
    assert!(!unlaunched(&row(
        json!("r"),
        "live",
        json!(null),
        json!(null)
    )));
    assert!(!unlaunched(&row(
        json!("r"),
        "dead",
        json!(null),
        json!(null)
    )));
}

#[test]
fn reservations_compare_as_python_compares_them() {
    let held = row(json!("r"), "live", json!(null), json!(null));
    assert!(holds_reservation(&held, &json!("r")));
    assert!(!holds_reservation(&held, &json!("R")));
    assert!(!holds_reservation(&held, &Value::Null));
    let null = row(json!(null), "live", json!(null), json!(null));
    assert!(holds_reservation(&null, &Value::Null));
    assert!(!holds_reservation(&null, &json!("")));
    let number = row(json!(5), "live", json!(null), json!(null));
    assert!(!holds_reservation(&number, &json!("5")));
    assert!(!holds_reservation(&number, &Value::Null));
}

#[test]
fn a_process_is_the_same_only_when_pid_and_start_time_are() {
    let recorded = |pid, started| row(json!("r"), "live", pid, started);
    assert!(same_process(
        &recorded(json!(7), json!("t")),
        &launch(7, "t")
    ));
    assert!(same_process(
        &recorded(json!(7.0), json!("t")),
        &launch(7, "t")
    ));
    for (pid, started) in [
        (json!(8), json!("t")),
        (json!(7), json!("u")),
        (json!(7.5), json!("t")),
        (json!("7"), json!("t")),
        (json!(7), json!(null)),
        (json!(null), json!("t")),
        (json!(9_007_199_254_740_992.0), json!("t")),
        (json!(1e300), json!("t")),
    ] {
        assert!(
            !same_process(&recorded(pid.clone(), started.clone()), &launch(7, "t")),
            "{pid} {started}"
        );
    }
    assert!(!same_process(
        &recorded(json!(9_007_199_254_740_992.0), json!("t")),
        &launch(9_007_199_254_740_993, "t")
    ));
    assert!(same_process(
        &recorded(json!(9_007_199_254_740_992.0), json!("t")),
        &launch(9_007_199_254_740_992, "t")
    ));
    assert!(!same_process(
        &recorded(json!(1e300), json!("t")),
        &launch(i64::MAX, "t")
    ));
    // 2^63 is the first float beyond i64: a saturating conversion would
    // make it i64::MAX, which Python's exact comparison never does
    // (#2271 round-1 review M2).
    let two_to_the_63 = 9_223_372_036_854_775_808.0_f64;
    assert!(!same_process(
        &recorded(json!(two_to_the_63), json!("t")),
        &launch(i64::MAX, "t")
    ));
    assert!(!same_process(
        &recorded(json!(-two_to_the_63 * 2.0), json!("t")),
        &launch(i64::MIN, "t")
    ));
    assert!(same_process(
        &recorded(json!(-two_to_the_63), json!("t")),
        &launch(i64::MIN, "t")
    ));
}

/// The launch identity is the caller's value as given, compared by
/// Python's `==` (#2271 round-1 review M1).
#[test]
fn a_loose_launch_identity_compares_as_python_compares_it() {
    let recorded = |pid, started| row(json!("r"), "live", pid, started);
    for (stored, given) in [
        ((json!(7), json!("t")), (json!(7.0), json!("t"))),
        ((json!(1), json!("t")), (json!(true), json!("t"))),
        ((json!(7), json!("5")), (json!(7), json!("5"))),
        ((json!(null), json!(null)), (json!(null), json!(null))),
    ] {
        assert!(
            same_process(
                &recorded(stored.0.clone(), stored.1.clone()),
                &loose(given.0.clone(), given.1.clone())
            ),
            "{stored:?} {given:?}"
        );
    }
    for (stored, given) in [
        ((json!(7), json!("t")), (json!("7"), json!("t"))),
        ((json!(7), json!("5")), (json!(7), json!(5))),
        ((json!(7), json!("t")), (json!([7]), json!("t"))),
        ((json!(null), json!("t")), (json!(0), json!("t"))),
    ] {
        assert!(
            !same_process(
                &recorded(stored.0.clone(), stored.1.clone()),
                &loose(given.0.clone(), given.1.clone())
            ),
            "{stored:?} {given:?}"
        );
    }
    let numeric = row(json!("5"), "live", json!(null), json!(null));
    assert!(holds_reservation(&numeric, &json!("5")));
    assert!(!holds_reservation(&numeric, &json!(5)));
}

#[test]
fn only_a_recorded_other_process_is_elsewhere() {
    let recorded = |pid| row(json!("r"), "live", pid, json!("t"));
    assert!(!launched_elsewhere(&recorded(json!(null)), &launch(7, "t")));
    assert!(!launched_elsewhere(&recorded(json!(7)), &launch(7, "t")));
    assert!(launched_elsewhere(&recorded(json!(8)), &launch(7, "t")));
    assert!(launched_elsewhere(&recorded(json!("abc")), &launch(7, "t")));
    let mut without = recorded(json!(8));
    without.columns.retain(|(name, _)| name != "pid");
    assert!(!launched_elsewhere(&without, &launch(7, "t")));
}
