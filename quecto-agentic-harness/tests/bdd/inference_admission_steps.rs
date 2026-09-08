//! Phase-local admission scenarios: no provider/runtime wiring.
use super::*;
use quecto::application::inference_admission::AdmissionService;
use quecto::application::ports::{AdmissionClient, AdmissionDispatcher, AdmissionRegistry};
use quecto::domain::inference_admission::{
    AdmissionConfig, Feedback, GroupId, GroupPolicy, RequestId, RequestState, ScopeId,
    TerminalOutcome, WorkloadClass,
};

#[derive(Debug, Default)]
pub struct AdmissionState {
    service: Option<AdmissionService>,
    child: Option<ScopeId>,
    observed: Vec<Option<RequestId>>,
    queued: Option<RequestState>,
    cancelled: Option<RequestState>,
}

fn group() -> GroupId {
    GroupId::new("test-budget").unwrap()
}

#[given("an admission group with one slot and an idle interactive parent")]
fn given_group(w: &mut QuectoWorld) {
    let mut s = AdmissionService::new(
        1,
        AdmissionConfig {
            groups: std::collections::BTreeMap::from([(
                group(),
                GroupPolicy {
                    capacity: 1,
                    reserve: 0,
                    min_interval_ms: 10,
                    queue_capacity: 4,
                    queue_timeout_ms: 100,
                    attempt_timeout_ms: 100,
                    fallback_base_ms: 1000,
                    max_cooldown_ms: 1000,
                },
            )]),
            aliases: std::collections::BTreeMap::from([("fixture".into(), group())]),
            max_scopes: 2,
            terminal_capacity: 4,
        },
    )
    .unwrap();
    let parent = s.register_root(WorkloadClass::Interactive).unwrap();
    w.admission.child = Some(s.register_child(parent).unwrap());
    w.admission.service = Some(s);
}

#[when("its background child queues an inference attempt")]
fn queue_child(w: &mut QuectoWorld) {
    let child = w.admission.child.unwrap();
    w.admission.queued = Some(
        w.admission
            .service
            .as_mut()
            .unwrap()
            .enqueue(child, 1, "fixture", 0)
            .unwrap(),
    );
}

#[then("the child is admitted without waiting for its parent")]
fn child_admitted(w: &mut QuectoWorld) {
    assert_eq!(
        w.admission.observed,
        vec![Some(RequestId {
            scope: w.admission.child.unwrap(),
            sequence: 1,
        })]
    );
}

#[when("the queued child inference is cancelled")]
fn cancel_child(w: &mut QuectoWorld) {
    w.admission.cancelled = Some(
        w.admission
            .service
            .as_mut()
            .unwrap()
            .cancel(w.admission.child.unwrap(), 1, 0)
            .unwrap(),
    );
}

#[then("no child inference is dispatched")]
fn no_dispatch(w: &mut QuectoWorld) {
    assert_eq!(
        w.admission.queued,
        Some(RequestState::Queued { deadline: 100 })
    );
    assert_eq!(
        w.admission.cancelled,
        Some(RequestState::Terminal(TerminalOutcome::Cancelled))
    );
    assert_eq!(w.admission.observed, vec![None]);
}

#[when("its background child completes one attempt and queues another")]
fn complete_and_queue(w: &mut QuectoWorld) {
    queue_child(w);
    let s = w.admission.service.as_mut().unwrap();
    let id = s.next(&group(), 0).unwrap().unwrap();
    s.complete(id.scope, id.sequence, Feedback::Success, 0)
        .unwrap();
    s.enqueue(id.scope, 2, "fixture", 0).unwrap();
}

#[then("the next attempt waits until the configured pacing boundary")]
fn pacing_boundary(w: &mut QuectoWorld) {
    assert_eq!(
        w.admission.observed,
        vec![
            None,
            Some(RequestId {
                scope: w.admission.child.unwrap(),
                sequence: 2,
            })
        ]
    );
}

#[when("admission is dispatched at time zero")]
fn dispatch_zero(w: &mut QuectoWorld) {
    w.admission.observed.push(
        w.admission
            .service
            .as_mut()
            .unwrap()
            .next(&group(), 0)
            .unwrap(),
    );
}

#[when("admission is dispatched after cancellation")]
fn dispatch_cancelled(w: &mut QuectoWorld) {
    w.admission.observed.push(
        w.admission
            .service
            .as_mut()
            .unwrap()
            .next(&group(), 10)
            .unwrap(),
    );
}

#[given("admission requests are paced ten milliseconds apart")]
fn given_pacing(w: &mut QuectoWorld) {
    given_group(w);
}

#[when("admission is attempted just before and at the pacing boundary")]
fn dispatch_boundary(w: &mut QuectoWorld) {
    for now in [9, 10] {
        w.admission.observed.push(
            w.admission
                .service
                .as_mut()
                .unwrap()
                .next(&group(), now)
                .unwrap(),
        );
    }
}
