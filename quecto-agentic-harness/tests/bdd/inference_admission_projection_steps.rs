//! P4 slice 2: the UDS `get_state` projection and `admission_state_changed`
//! push of a process's admission activity, over a real in-process authority.
//! The observation steps supply the queued attempt; these steps only look at
//! it the way a socket client does.

use super::inference_admission_authority_steps::{LIMIT, rt};
use super::*;
use quecto::application::ports::AdmissionObservation;
use quecto::infrastructure::admission::AdmissionRecorder;
use quecto::interface::cli::{AdmissionStateProbe, admission_broadcast_hook};
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
pub struct ProjectionState {
    probe: Option<AdmissionStateProbe>,
    last: Option<serde_json::Value>,
    generation: Option<u64>,
    events: Option<tokio::sync::broadcast::Receiver<String>>,
}
impl std::fmt::Debug for ProjectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<admission projection scenario>")
    }
}

fn probe(world: &mut QuectoWorld) -> &mut AdmissionStateProbe {
    if world.admission_projection.probe.is_none() {
        let recorder = world
            .authority_observation
            .recorder
            .clone()
            .expect("an observed attempt was queued first");
        let mut probe = AdmissionStateProbe::new(recorder);
        probe.start_run();
        world.admission_projection.probe = Some(probe);
    }
    world.admission_projection.probe.as_mut().unwrap()
}

#[when("a supervisor polls the process state")]
fn when_polls(world: &mut QuectoWorld) {
    let data = probe(world).poll(None);
    let p = &mut world.admission_projection;
    p.generation = data["generation"].as_u64();
    p.last = Some(data);
}

#[when("the supervisor polls again with the generation it last saw")]
fn when_polls_since(world: &mut QuectoWorld) {
    let since = world.admission_projection.generation;
    let data = probe(world).poll(since);
    if let Some(generation) = data["generation"].as_u64() {
        world.admission_projection.generation = Some(generation);
    }
    world.admission_projection.last = Some(data);
}

#[then(expr = "the state is {string} with progress {string} naming group {string}")]
fn then_state_and_progress(
    world: &mut QuectoWorld,
    state: String,
    progress: String,
    group: String,
) {
    let data = world.admission_projection.last.as_ref().unwrap();
    assert_eq!(data["state"], state, "{data}");
    assert_eq!(data["progress"]["state"], progress, "{data}");
    let reason = data["progress"]["reason"].as_str().unwrap();
    if group.is_empty() {
        assert!(!reason.contains("waiting for admission"), "{reason}");
    } else {
        assert!(reason.contains(&format!("group {group}")), "{reason}");
    }
}

#[then(expr = "the admission view shows {int} waiting and {int} admitted")]
fn then_admission_counts(world: &mut QuectoWorld, waiting: u64, admitted: u64) {
    let data = world.admission_projection.last.as_ref().unwrap();
    assert_eq!(data["admission"]["waiting"], waiting, "{data}");
    assert_eq!(data["admission"]["admitted"], admitted, "{data}");
}

#[then("the response is the unchanged marker")]
fn then_unchanged(world: &mut QuectoWorld) {
    let data = world.admission_projection.last.as_ref().unwrap();
    assert_eq!(data["unchanged"], true, "{data}");
    assert_eq!(
        data["generation"].as_u64(),
        world.admission_projection.generation
    );
}

#[then(expr = "the response is a changed view with progress {string}")]
fn then_changed(world: &mut QuectoWorld, progress: String) {
    let data = world.admission_projection.last.as_ref().unwrap();
    assert!(data.get("unchanged").is_none(), "{data}");
    assert_eq!(data["progress"]["state"], progress, "{data}");
}

#[when("the waiting run is aborted")]
fn when_aborted(world: &mut QuectoWorld) {
    let pending = world.authority_observation.pending.take().unwrap();
    pending.abort();
    let outcome =
        rt(&world.authority).block_on(async { tokio::time::timeout(LIMIT, pending).await });
    assert!(
        outcome
            .expect("join within the limit")
            .unwrap_err()
            .is_cancelled(),
        "the acquire was aborted, not completed"
    );
    let recorder = world.authority_observation.recorder.clone().unwrap();
    let start = std::time::Instant::now();
    while recorder.snapshot().waiting > 0 {
        assert!(start.elapsed() < LIMIT, "abort never cancelled the wait");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[then(expr = "the admission view counts {int} cancelled attempt and nothing waiting")]
fn then_cancelled(world: &mut QuectoWorld, cancelled: u64) {
    let data = world.admission_projection.last.as_ref().unwrap();
    assert_eq!(
        data["admission"]["counters"]["cancelled"], cancelled,
        "{data}"
    );
    assert_eq!(data["admission"]["waiting"], 0, "{data}");
    assert!(
        data["admission"].get("longestWaitSeconds").is_none(),
        "{data}"
    );
}

#[when("the process pushes admission transitions to its socket clients")]
fn when_hook_installed(world: &mut QuectoWorld) {
    let (tx, rx) = tokio::sync::broadcast::channel::<String>(64);
    let recorder = Arc::new(AdmissionRecorder::new());
    recorder.set_hook(admission_broadcast_hook(tx));
    world.authority_observation.recorder = Some(recorder);
    world.admission_projection.events = Some(rx);
}

#[then(expr = "the clients receive admission_state_changed events ending with {int} completed")]
fn then_events(world: &mut QuectoWorld, completed: u64) {
    let rx = world.admission_projection.events.as_mut().unwrap();
    let mut revisions = Vec::new();
    let mut first = None;
    let mut last = None;
    while let Ok(line) = rx.try_recv() {
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(event["type"], "admission_state_changed", "{event}");
        revisions.push(event["admission"]["revision"].as_u64().unwrap());
        first.get_or_insert_with(|| event.clone());
        last = Some(event);
    }
    assert!(revisions.windows(2).all(|w| w[1] > w[0]), "{revisions:?}");
    let first = first.expect("the queued transition was pushed");
    assert_eq!(first["admission"]["waiting"], 1, "{first}");
    assert_eq!(first["admission"]["revision"], 1, "{first}");
    assert_eq!(
        revisions.len(),
        3,
        "queued, granted, completed: {revisions:?}"
    );
    let last = last.expect("at least one transition was pushed");
    assert_eq!(
        last["admission"]["counters"]["completed"], completed,
        "{last}"
    );
    assert_eq!(last["admission"]["waiting"], 0, "{last}");
}
