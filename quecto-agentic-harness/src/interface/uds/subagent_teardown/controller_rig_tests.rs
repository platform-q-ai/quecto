//! Shared controller test rig: fakes wired through the real use cases.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::*;
use crate::application::subagents::ports::SubagentLifecycleRepository;
use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::application::subagents::use_cases::{
    ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
};
use crate::domain::subagent_teardown::{HarnessLifecycleState, LineageSnapshot};
use crate::interface::uds::subagent_teardown::ack_fakes::RecordingWriter;

pub(super) struct Rig {
    pub(super) lifecycle: Arc<FakeLifecycle>,
    pub(super) routing: Arc<FakeRouting>,
    pub(super) cancellation: Arc<FakeCancellation>,
    pub(super) persistence: Arc<FakePersistence>,
    pub(super) exit: Arc<FakeExit>,
    pub(super) spawner: Arc<FakeSpawner>,
    pub(super) controller: Arc<SubagentTeardownController>,
}

pub(super) fn rig_with(lineage: LineageSnapshot, cancellation: Arc<FakeCancellation>) -> Rig {
    let lifecycle = FakeLifecycle::new(lineage);
    let routing = FakeRouting::new();
    let persistence = FakePersistence::new();
    let exit = FakeExit::new();
    let spawner = FakeSpawner::new();
    let transaction = HarnessShutdownTransaction::new(lifecycle.clone(), FakeClock::at(42));
    let prepare = Arc::new(PrepareHarnessShutdown::new(transaction.clone()));
    let execute = Arc::new(ExecuteHarnessShutdown::new(
        transaction,
        ExecuteHarnessShutdownPorts {
            routing: routing.clone(),
            cancellation: cancellation.clone(),
            persistence: persistence.clone(),
            exit: exit.clone(),
            spawner: spawner.clone(),
        },
    ));
    let terminate = Arc::new(TerminateDelegatedAgent::new(
        lifecycle.clone(),
        routing.clone(),
    ));
    Rig {
        lifecycle,
        routing,
        cancellation,
        persistence,
        exit,
        spawner,
        controller: Arc::new(SubagentTeardownController::new(prepare, execute, terminate)),
    }
}

pub(super) fn rig_for(lineage: LineageSnapshot) -> Rig {
    rig_with(lineage, FakeCancellation::new(true))
}

pub(super) fn rig() -> Rig {
    rig_for(root_tree())
}

/// A writer whose ordering trace the cancellation fake also writes into, so
/// "execute-started" before "ack-flushed" would be visible.
pub(super) fn traced_writer(rig: &Rig) -> RecordingWriter {
    let writer = RecordingWriter::default();
    *rig.cancellation.trace.lock().unwrap() = Some(writer.trace.clone());
    writer
}

impl Rig {
    pub(super) fn nothing_ran(&self) {
        assert_eq!(self.spawner.spawned.load(Ordering::SeqCst), 0);
        assert!(self.routing.calls().is_empty());
        assert_eq!(self.cancellation.calls.load(Ordering::SeqCst), 0);
        assert!(self.persistence.calls.lock().unwrap().is_empty());
        assert!(self.exit.signalled.lock().unwrap().is_empty());
        assert_eq!(self.lifecycle.lifecycle(), HarnessLifecycleState::Accepting);
    }

    pub(super) async fn handle(
        &self,
        line: &str,
        authority: ConnectionAuthority,
        delivery: DeliveryState,
        writer: &RecordingWriter,
    ) -> ControllerOutcome {
        self.controller
            .handle(line, authority, delivery, writer)
            .await
    }
}

pub(super) const SHUTDOWN_LINE: &str =
    r#"{"type":"shutdown","id":"c-1","reason":"parent_shutdown"}"#;

pub(super) fn frame_json(frame: &str) -> serde_json::Value {
    serde_json::from_str(frame.trim_end()).unwrap()
}
