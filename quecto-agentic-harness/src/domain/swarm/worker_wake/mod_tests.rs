use super::*;

fn board(status: RunStatus, ready: i64, claimed_by: &[&str]) -> WorkerBoard {
    WorkerBoard {
        status,
        ready,
        claimed_by: claimed_by.iter().map(|owner| (*owner).to_owned()).collect(),
    }
}

fn working(members: &[&str]) -> BTreeSet<String> {
    members.iter().map(|member| (*member).to_owned()).collect()
}

#[test]
fn a_claim_held_by_a_worker_that_is_not_working_is_stranded() {
    for status in [RunStatus::Setup, RunStatus::Running] {
        assert_eq!(
            stranded_work(&board(status, 0, &["b", "a"]), &working(&["a"])),
            Some(Stranded {
                claimed_by: vec!["b".into()],
                ready: 0
            })
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
        Some(Stranded {
            claimed_by: vec![],
            ready: 2
        })
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
fn the_same_stranded_work_has_the_same_signature() {
    let one = Stranded {
        claimed_by: vec!["b".into()],
        ready: 1,
    };
    assert_eq!(one.signature(), one.clone().signature());
    let other = Stranded {
        claimed_by: vec!["c".into()],
        ready: 1,
    };
    assert_ne!(one.signature(), other.signature());
}

#[test]
fn a_reply_or_the_first_turn_after_a_failure_is_the_ordinary_note() {
    use WorkerTurnEnd::*;
    for (reply_due, after_failure, settled) in [
        (false, false, CheckBoard),
        (true, false, Ordinary),
        (false, true, Ordinary),
        (true, true, Ordinary),
    ] {
        assert_eq!(
            settle_worker_turn_end(reply_due, after_failure),
            settled,
            "{reply_due} {after_failure}"
        );
    }
}
