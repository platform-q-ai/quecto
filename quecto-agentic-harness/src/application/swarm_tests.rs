use super::*;
use crate::domain::subagent_launch::LaunchFuture;
use crate::domain::swarm::{Member, ProcessIdentity, RunStatus};
use std::sync::Mutex;

struct Processes {
    events: Mutex<Vec<String>>,
    accepts: bool,
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
    fn abort<'a>(&'a self, member: &'a Member) -> LaunchFuture<'a, bool> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push(format!("abort:{}", member.id));
            self.accepts
        })
    }
    fn terminate<'a>(
        &'a self,
        process: &'a ProcessIdentity,
    ) -> LaunchFuture<'a, Result<(), DomainError>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push(format!("terminate:{}", process.pid));
            Ok(())
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
        };
        settle(&snapshot(status), "parent", &processes)
            .await
            .unwrap();
        let mut expected = vec!["cancel-jobs", "abort:worker", "terminate:2"];
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
    };
    settle(&snapshot(RunStatus::Failed), "parent", &processes)
        .await
        .unwrap();
    assert_eq!(
        *processes.events.lock().unwrap(),
        [
            "cancel-jobs",
            "suspend-local",
            "abort:worker",
            "terminate:2"
        ]
    );
}
#[tokio::test]
async fn active_runs_have_no_settlement_effects() {
    for status in [RunStatus::Setup, RunStatus::Running] {
        let processes = Processes {
            events: Mutex::new(vec![]),
            accepts: false,
        };
        settle(&snapshot(status), "parent", &processes)
            .await
            .unwrap();
        assert!(processes.events.lock().unwrap().is_empty());
    }
}

struct Time(f64);
impl crate::domain::swarm::Clock for Time {
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
    };
    settle(&snapshot(RunStatus::Paused), "parent", &processes)
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
        };
        settle(&snapshot(status), "parent", &processes)
            .await
            .unwrap();
        let events = processes.events.lock().unwrap();
        assert!(
            !events.iter().any(|event| event == "terminate:1"),
            "coordinator must remain available for reports: {events:?}"
        );
        assert!(
            events.iter().any(|event| event == "terminate:2"),
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
    };
    settle(&ended, "parent", &coordinator).await.unwrap();
    assert_eq!(
        *coordinator.events.lock().unwrap(),
        ["suspend-jobs"],
        "the registry stays open for the resume; the turn keeps reporting"
    );
    let worker = Processes {
        events: Mutex::new(vec![]),
        accepts: true,
    };
    settle(&ended, "worker", &worker).await.unwrap();
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
    };
    settle(&plain, "parent", &processes).await.unwrap();
    assert_eq!(
        *processes.events.lock().unwrap(),
        ["suspend-jobs", "suspend-local"]
    );
    // Only proposable outcomes end a run.
    let mut odd = snapshot(RunStatus::Paused);
    odd.outcome = Some(RunStatus::Cancelled);
    assert!(!odd.ended());
}
