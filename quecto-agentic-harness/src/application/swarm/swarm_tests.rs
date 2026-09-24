use super::ports::PortFuture;
use super::*;
use crate::domain::swarm::{Member, MemberExit, ProcessIdentity, RunStatus};
use std::sync::Mutex;

struct Processes {
    events: Mutex<Vec<String>>,
    accepts: bool,
    fail_terminate: bool,
}
impl ProcessControl for Processes {
    fn suspend_local_executions(&self, _: &Snapshot) {
        self.events.lock().unwrap().push("suspend-jobs".into());
    }
    fn suspend_local_inference(&self, _: &Snapshot) {
        self.events.lock().unwrap().push("suspend-local".into());
    }
    fn cancel_local_executions(&self) {
        self.events.lock().unwrap().push("cancel-jobs".into());
    }
    fn abort<'a>(&'a self, member: &'a Member) -> PortFuture<'a, bool> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push(format!("abort:{}", member.id));
            self.accepts
        })
    }
    fn terminate<'a>(&'a self, member: &'a Member) -> PortFuture<'a, Result<(), DomainError>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push(format!("terminate:{}", member.id));
            if self.fail_terminate {
                Err(DomainError::Tool(format!("{} unreachable", member.id)))
            } else {
                Ok(())
            }
        })
    }
}
fn snapshot(status: RunStatus) -> Snapshot {
    Snapshot {
        control_generation: 0,
        status,
        outcome: None,
        coordinator: "parent".into(),
        deadline: 100.,
        members: ["parent", "worker"]
            .into_iter()
            .enumerate()
            .map(|(i, id)| Member {
                id: id.into(),
                status: MemberStatus::Live,
                process: Some(ProcessIdentity {
                    pid: i as u32 + 1,
                    started: "identity".into(),
                }),
                endpoint: Some(id.into()),
            })
            .collect(),
    }
}
#[tokio::test]
async fn terminal_settlement_cancels_jobs_before_any_abort_and_preserves_reporting_coordinator() {
    for status in [
        RunStatus::Succeeded,
        RunStatus::Cancelled,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::BudgetExhausted,
    ] {
        let processes = Processes {
            events: Mutex::new(vec![]),
            accepts: true,
            fail_terminate: false,
        };
        settle(&snapshot(status), "parent", &processes, &AllAlive)
            .await
            .unwrap();
        let mut expected = vec!["cancel-jobs", "abort:worker", "terminate:worker"];
        if status.abort_coordinator() {
            expected.insert(1, "suspend-local");
        }
        assert_eq!(*processes.events.lock().unwrap(), expected);
    }
}
#[tokio::test]
async fn failed_worker_abort_still_terminates_worker_and_suspends_local_coordinator() {
    let processes = Processes {
        events: Mutex::new(vec![]),
        accepts: false,
        fail_terminate: false,
    };
    settle(
        &snapshot(RunStatus::Failed),
        "parent",
        &processes,
        &AllAlive,
    )
    .await
    .unwrap();
    assert_eq!(
        *processes.events.lock().unwrap(),
        [
            "cancel-jobs",
            "suspend-local",
            "abort:worker",
            "terminate:worker"
        ]
    );
}
#[tokio::test]
async fn active_runs_have_no_settlement_effects() {
    for status in [RunStatus::Setup, RunStatus::Running] {
        let processes = Processes {
            events: Mutex::new(vec![]),
            accepts: false,
            fail_terminate: false,
        };
        settle(&snapshot(status), "parent", &processes, &AllAlive)
            .await
            .unwrap();
        assert!(processes.events.lock().unwrap().is_empty());
    }
}

struct Time(f64);
impl crate::application::swarm::ports::Clock for Time {
    fn now_seconds(&self) -> f64 {
        self.0
    }
}
#[test]
fn deadline_and_terminal_outcomes_trigger_settlement_with_a_fake_clock() {
    assert!(!settlement_due(&snapshot(RunStatus::Running), &Time(99.)));
    // A passed deadline is a resumable pause, not a settlement (#1729).
    assert_eq!(
        observed_outcome(&snapshot(RunStatus::Running), &Time(100.)),
        RunStatus::Paused
    );
    assert!(!settlement_due(&snapshot(RunStatus::Running), &Time(100.)));
    assert!(settlement_due(&snapshot(RunStatus::Failed), &Time(0.)));
    assert!(!settlement_due(&snapshot(RunStatus::Setup), &Time(1000.)));
    assert!(!settlement_due(&snapshot(RunStatus::Paused), &Time(1000.)));
}

struct CoordinationFake(Mutex<Vec<String>>);
impl CoordinationPort for CoordinationFake {
    fn snapshot(&self) -> Result<Snapshot, DomainError> {
        Ok(snapshot(RunStatus::Running))
    }
    fn register_endpoint(&self, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }
    fn reserve_member(&self, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }
    fn record_launch(&self, _: &str, _: &str, _: &ProcessIdentity) -> Result<(), DomainError> {
        unreachable!()
    }
    fn confirm_unlaunched(&self, _: &str) -> Result<(), DomainError> {
        panic!("harness death must never release ownership")
    }
    fn quarantine(&self, member: &str) -> Result<(), DomainError> {
        self.0.lock().unwrap().push(member.into());
        Ok(())
    }
    fn confirm_dead(&self, member: &str, _: MemberExit) -> Result<(), DomainError> {
        self.0.lock().unwrap().push(format!("dead:{member}"));
        Ok(())
    }
}
struct Observations;
impl ProcessObservation for Observations {
    fn harness_dead(&self, process: &ProcessIdentity) -> bool {
        process.pid == 2
    }
}
#[test]
fn harness_death_quarantines_without_releasing_ownership() {
    let coordination = CoordinationFake(Mutex::new(vec![]));
    reconcile(&coordination, &Observations).unwrap();
    assert_eq!(*coordination.0.lock().unwrap(), ["worker"]);
}

#[tokio::test]
async fn suspension_cancels_only_local_work_without_terminating_members() {
    let processes = Processes {
        events: Mutex::new(vec![]),
        accepts: false,
        fail_terminate: false,
    };
    settle(
        &snapshot(RunStatus::Paused),
        "parent",
        &processes,
        &AllAlive,
    )
    .await
    .unwrap();
    assert_eq!(
        *processes.events.lock().unwrap(),
        ["suspend-jobs", "suspend-local"]
    );
}

#[tokio::test]
async fn failed_or_expired_run_retains_coordinator_when_abort_delivery_fails() {
    for status in [RunStatus::Failed, RunStatus::BudgetExhausted] {
        let processes = Processes {
            events: Mutex::new(vec![]),
            accepts: false,
            fail_terminate: false,
        };
        settle(&snapshot(status), "parent", &processes, &AllAlive)
            .await
            .unwrap();
        let events = processes.events.lock().unwrap();
        assert!(
            !events.iter().any(|event| event == "terminate:parent"),
            "coordinator must remain available for reports: {events:?}"
        );
        assert!(
            events.iter().any(|event| event == "terminate:worker"),
            "worker cleanup still required"
        );
    }
}

/// #1729: a run its coordinator ended is a pause that keeps the coordinator
/// reporting (its finished interpreter is suspended, its turn is not) while
/// every other member suspends intact; nobody is aborted or terminated.
#[tokio::test]
async fn an_ended_run_settles_as_a_pause_that_keeps_the_coordinator_reporting() {
    let mut ended = snapshot(RunStatus::Paused);
    ended.outcome = Some(RunStatus::Succeeded);
    assert!(ended.ended());
    assert!(ended.admits_inference("parent"));
    assert!(!ended.admits_inference("worker"));
    let coordinator = Processes {
        events: Mutex::new(vec![]),
        accepts: true,
        fail_terminate: false,
    };
    settle(&ended, "parent", &coordinator, &AllAlive)
        .await
        .unwrap();
    assert_eq!(
        *coordinator.events.lock().unwrap(),
        ["suspend-jobs"],
        "the registry stays open for the resume; the turn keeps reporting"
    );
    let worker = Processes {
        events: Mutex::new(vec![]),
        accepts: true,
        fail_terminate: false,
    };
    settle(&ended, "worker", &worker, &AllAlive).await.unwrap();
    assert_eq!(
        *worker.events.lock().unwrap(),
        ["suspend-jobs", "suspend-local"]
    );
    // A plain pause suspends the coordinator too, and admits nobody.
    let plain = snapshot(RunStatus::Paused);
    assert!(!plain.ended() && !plain.admits_inference("parent"));
    let processes = Processes {
        events: Mutex::new(vec![]),
        accepts: true,
        fail_terminate: false,
    };
    settle(&plain, "parent", &processes, &AllAlive)
        .await
        .unwrap();
    assert_eq!(
        *processes.events.lock().unwrap(),
        ["suspend-jobs", "suspend-local"]
    );
    // Only proposable outcomes end a run.
    let mut odd = snapshot(RunStatus::Paused);
    odd.outcome = Some(RunStatus::Cancelled);
    assert!(!odd.ended());
}

/// #1939: a member that cannot be ended by delegation is reported after
/// every other member was still asked; the member's process identity is
/// never what the settlement acts on (the port receives the member).
#[tokio::test]
async fn an_unreachable_member_is_reported_after_the_others_were_asked() {
    let processes = Processes {
        events: Mutex::new(vec![]),
        accepts: true,
        fail_terminate: true,
    };
    let mut snapshot = snapshot(RunStatus::Cancelled);
    snapshot.members.push(Member {
        id: "second".into(),
        status: MemberStatus::Live,
        process: None,
        endpoint: None,
    });
    let error = settle(&snapshot, "parent", &processes, &AllAlive)
        .await
        .expect_err("an unreachable member is a truthful failure");
    let text = error.to_string();
    assert!(text.contains("worker unreachable"), "{text}");
    assert!(text.contains("second unreachable"), "{text}");
    assert_eq!(
        *processes.events.lock().unwrap(),
        [
            "cancel-jobs",
            "abort:worker",
            "terminate:worker",
            "abort:second",
            "terminate:second"
        ],
        "a member without a process identity is still asked"
    );
}

/// A board whose members die when their death is confirmed, so the
/// reconcile that follows a confirmed exit sees them already dead.
struct ExitBoard {
    log: Mutex<Vec<String>>,
    dead: Mutex<Vec<String>>,
}
impl CoordinationPort for ExitBoard {
    fn snapshot(&self) -> Result<Snapshot, DomainError> {
        let mut snapshot = snapshot(RunStatus::Running);
        let dead = self.dead.lock().unwrap();
        for member in &mut snapshot.members {
            if dead.contains(&member.id) {
                member.status = MemberStatus::Dead;
            }
        }
        Ok(snapshot)
    }
    fn register_endpoint(&self, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }
    fn reserve_member(&self, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }
    fn record_launch(&self, _: &str, _: &str, _: &ProcessIdentity) -> Result<(), DomainError> {
        unreachable!()
    }
    fn confirm_unlaunched(&self, _: &str) -> Result<(), DomainError> {
        panic!("an exit never releases an unlaunched reservation")
    }
    fn quarantine(&self, member: &str) -> Result<(), DomainError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("quarantine:{member}"));
        Ok(())
    }
    fn confirm_dead(&self, member: &str, exit: MemberExit) -> Result<(), DomainError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("dead:{member}:{}", exit.as_str()));
        self.dead.lock().unwrap().push(member.into());
        Ok(())
    }
}
struct AllAlive;
impl ProcessObservation for AllAlive {
    fn harness_dead(&self, _: &ProcessIdentity) -> bool {
        false
    }
}

#[test]
fn an_observed_member_exit_confirms_death_instead_of_quarantining_the_run() {
    let board = ExitBoard {
        log: Mutex::new(vec![]),
        dead: Mutex::new(vec![]),
    };
    // The worker's process (pid 2) is gone by the time the reaper runs.
    let snapshot = member_exited(&board, &Observations, "worker", MemberExit::Abrupt).unwrap();
    assert_eq!(*board.log.lock().unwrap(), ["dead:worker:abrupt"]);
    assert_eq!(
        snapshot
            .members
            .iter()
            .find(|m| m.id == "worker")
            .unwrap()
            .status,
        MemberStatus::Dead
    );
    // A second observation of the same exit changes nothing.
    member_exited(&board, &Observations, "worker", MemberExit::Orderly).unwrap();
    assert_eq!(*board.log.lock().unwrap(), ["dead:worker:abrupt"]);
}

#[test]
fn a_socket_loss_without_an_observed_exit_never_confirms_death() {
    let board = ExitBoard {
        log: Mutex::new(vec![]),
        dead: Mutex::new(vec![]),
    };
    // The monitor's connection-level observation reaches only `reconcile`,
    // and a member whose harness is still alive is left exactly as it is.
    reconcile(&board, &AllAlive).unwrap();
    assert!(board.log.lock().unwrap().is_empty());
    // A vanished harness that nobody reaped is still a quarantine, never a
    // confirmed death.
    reconcile(&board, &Observations).unwrap();
    assert_eq!(*board.log.lock().unwrap(), ["quarantine:worker"]);
}

/// The coordinator's harness (pid 1) is gone; every other harness is alive.
struct CoordinatorGone;
impl ProcessObservation for CoordinatorGone {
    fn harness_dead(&self, process: &ProcessIdentity) -> bool {
        process.pid == 1
    }
}

fn recording() -> Processes {
    Processes {
        events: Mutex::new(vec![]),
        accepts: true,
        fail_terminate: false,
    }
}

#[tokio::test]
async fn a_member_settling_a_closed_run_ends_nobody_and_leaves_ending_to_the_coordinator() {
    // #2121: members that ended each other (and themselves) over their
    // sockets bypassed the launcher, which then reported every exit as
    // unexpected. Only the coordinator, whose harness launched them, ends
    // members; a member stops only its own work.
    for status in [
        RunStatus::Succeeded,
        RunStatus::Cancelled,
        RunStatus::Failed,
    ] {
        let processes = recording();
        settle(&snapshot(status), "worker", &processes, &AllAlive)
            .await
            .unwrap();
        assert_eq!(
            *processes.events.lock().unwrap(),
            ["cancel-jobs", "suspend-local"],
            "{status:?}"
        );
    }
}

#[tokio::test]
async fn members_end_the_run_themselves_once_the_coordinator_is_gone() {
    let processes = recording();
    settle(
        &snapshot(RunStatus::Failed),
        "worker",
        &processes,
        &CoordinatorGone,
    )
    .await
    .unwrap();
    assert_eq!(
        *processes.events.lock().unwrap(),
        ["cancel-jobs", "abort:worker", "terminate:worker"]
    );

    let mut dead = snapshot(RunStatus::Failed);
    dead.members[0].status = MemberStatus::Dead;
    let processes = recording();
    settle(&dead, "worker", &processes, &AllAlive)
        .await
        .unwrap();
    assert_eq!(
        *processes.events.lock().unwrap(),
        ["cancel-jobs", "abort:worker", "terminate:worker"]
    );
}
