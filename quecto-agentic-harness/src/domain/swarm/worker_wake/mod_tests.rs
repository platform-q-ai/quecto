use super::*;

fn board(status: RunStatus, ready: i64, claimed_by: &[&str]) -> WorkerBoard {
    WorkerBoard {
        status,
        ready,
        claimed_by: claimed_by.iter().map(|owner| (*owner).to_owned()).collect(),
        coordinator: "coordinator".into(),
    }
}

fn working(members: &[&str]) -> BTreeSet<String> {
    members.iter().map(|member| (*member).to_owned()).collect()
}

fn stranded(claimed_by: &[&str], ready: i64) -> Stranded {
    Stranded {
        claimed_by: claimed_by.iter().map(|owner| (*owner).to_owned()).collect(),
        ready,
    }
}

#[test]
fn a_claim_held_by_a_worker_that_is_not_working_is_stranded() {
    for status in [RunStatus::Setup, RunStatus::Running] {
        assert_eq!(
            stranded_work(&board(status, 0, &["b", "a", "b"]), &working(&["a"])),
            Some(stranded(&["b", "b"], 0))
        );
    }
}

#[test]
fn claims_whose_owners_all_work_are_not_stranded() {
    assert_eq!(
        stranded_work(
            &board(RunStatus::Running, 0, &["a", "b"]),
            &working(&["a", "b"])
        ),
        None
    );
}

#[test]
fn ready_work_is_stranded_only_when_no_worker_works() {
    assert_eq!(
        stranded_work(&board(RunStatus::Running, 2, &[]), &working(&[])),
        Some(stranded(&[], 2))
    );
    assert_eq!(
        stranded_work(&board(RunStatus::Running, 2, &[]), &working(&["a"])),
        None
    );
}

#[test]
fn nothing_left_or_a_run_that_is_not_live_strands_nothing() {
    assert_eq!(
        stranded_work(&board(RunStatus::Running, 0, &[]), &working(&[])),
        None
    );
    for status in [
        RunStatus::Paused,
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::BudgetExhausted,
    ] {
        assert_eq!(
            stranded_work(&board(status, 3, &["b"]), &working(&[])),
            None,
            "{status:?}"
        );
    }
}

#[test]
fn a_report_covers_the_same_or_less_and_not_a_new_claim_or_more_ready_work() {
    let reported = stranded(&["a", "b"], 1);
    assert!(reported.covers(&stranded(&["a", "b"], 1)));
    assert!(reported.covers(&stranded(&["a"], 0)));
    assert!(!reported.covers(&stranded(&["a", "c"], 0)), "a new claim");
    assert!(
        !reported.covers(&stranded(&["a", "a"], 0)),
        "a second claim"
    );
    assert!(!reported.covers(&stranded(&["a"], 2)), "more ready work");
}

#[test]
fn a_report_is_rearmed_by_a_worker_it_names_or_any_worker_for_ready_work() {
    assert!(stranded(&["a"], 0).rearmed_by("a"));
    assert!(!stranded(&["a"], 0).rearmed_by("b"));
    assert!(stranded(&[], 1).rearmed_by("b"));
}

#[test]
fn the_coordinator_s_own_claim_is_not_stranded() {
    assert_eq!(
        stranded_work(
            &board(RunStatus::Running, 0, &["coordinator"]),
            &working(&[])
        ),
        None
    );
}
