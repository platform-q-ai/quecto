use quecto::application::swarm::LifecycleService;
use quecto::domain::error::DomainError;
use quecto::domain::subagent_launch::LaunchFuture;
use quecto::domain::swarm::{
    Clock, CoordinationPort, Member, MemberStatus, ProcessControl, ProcessIdentity,
    ProcessObservation, RunStatus, Snapshot, SwarmLifecycle,
};
use std::sync::Mutex;

pub(super) fn snapshot() -> Snapshot {
    Snapshot {
        outcome: None,
        control_generation: 0,
        status: RunStatus::Running,
        coordinator: "worker".into(),
        deadline: 100.,
        members: vec![Member {
            id: "worker".into(),
            status: MemberStatus::Live,
            endpoint: Some("opaque-endpoint".into()),
            process: Some(ProcessIdentity {
                pid: 42,
                started: "generation".into(),
            }),
        }],
    }
}
pub(super) struct Effects(pub Mutex<Vec<&'static str>>);
impl ProcessControl for Effects {
    fn suspend_local_executions(&self, _: &Snapshot) {
        self.0.lock().unwrap().push("suspend");
    }
    fn suspend_local_inference(&self, _: &Snapshot) {}
    fn cancel_local_executions(&self) {
        self.0.lock().unwrap().push("cancel");
    }
    fn abort<'a>(&'a self, _: &'a Member) -> LaunchFuture<'a, bool> {
        Box::pin(async {
            self.0.lock().unwrap().push("abort");
            false
        })
    }
    fn terminate<'a>(
        &'a self,
        _: &'a ProcessIdentity,
    ) -> LaunchFuture<'a, Result<(), DomainError>> {
        Box::pin(async {
            self.0.lock().unwrap().push("terminate");
            Ok(())
        })
    }
}
pub(super) struct Time(pub f64);
impl Clock for Time {
    fn now_seconds(&self) -> f64 {
        self.0
    }
}
pub(super) struct Observation;
impl ProcessObservation for Observation {
    fn harness_dead(&self, p: &ProcessIdentity) -> bool {
        p.started == "generation"
    }
}
pub(super) struct Board(pub Mutex<Vec<String>>);
impl CoordinationPort for Board {
    fn snapshot(&self) -> Result<Snapshot, DomainError> {
        Ok(snapshot())
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
        panic!("must not release uncertain scope")
    }
    fn quarantine(&self, member: &str) -> Result<(), DomainError> {
        self.0.lock().unwrap().push(member.into());
        Ok(())
    }
}

#[tokio::test]
async fn injected_application_service_runs_through_its_public_contract() {
    let service: &dyn SwarmLifecycle = &LifecycleService;
    let mut snapshot = snapshot();
    snapshot.coordinator = "parent".into();
    let effects = Effects(Mutex::new(vec![]));
    service.settle(&snapshot, "parent", &effects).await.unwrap();
    assert!(effects.0.lock().unwrap().is_empty());
    snapshot.status = service.observed_outcome(&snapshot, &Time(100.));
    assert_eq!(snapshot.status, RunStatus::Paused);
    service.settle(&snapshot, "parent", &effects).await.unwrap();
    assert_eq!(*effects.0.lock().unwrap(), ["suspend"]);
    snapshot.status = RunStatus::Cancelled;
    service.settle(&snapshot, "parent", &effects).await.unwrap();
    assert_eq!(
        *effects.0.lock().unwrap(),
        ["suspend", "cancel", "abort", "terminate"]
    );
    let board = Board(Mutex::new(vec![]));
    service.reconcile(&board, &Observation).unwrap();
    assert_eq!(*board.0.lock().unwrap(), ["worker"]);
}
