use super::uds_admission_projection::{admission_event_hook, project, waiting_progress};
use super::uds_execution_state::ExecutionState;
use super::uds_state_projection::{slim_state_projection, slim_state_response_data};
use crate::application::ports::AdmissionObservation;
use crate::domain::inference_admission::{
    AdmissionActivity, AdmissionPhase, AttemptObservation, CooldownState, GroupActivity, GroupId,
};
use std::sync::{Arc, Mutex};

/// A scripted read port: tests move it through revisions by hand.
#[derive(Default)]
pub(crate) struct ScriptedObservation(Mutex<AdmissionActivity>);

impl ScriptedObservation {
    pub(crate) fn set(&self, activity: AdmissionActivity) {
        *self.0.lock().unwrap() = activity;
    }
}

impl AdmissionObservation for ScriptedObservation {
    fn snapshot(&self) -> AdmissionActivity {
        self.0.lock().unwrap().clone()
    }
}

fn g(name: &str) -> GroupId {
    GroupId::new(name).unwrap()
}

pub(crate) fn waiting_activity(revision: u64, elapsed_ms: u64) -> AdmissionActivity {
    let mut activity = AdmissionActivity {
        waiting: 1,
        revision,
        observed_at_ms: 10_000,
        ..AdmissionActivity::default()
    };
    activity.groups.insert(g("g"), GroupActivity::default());
    activity.attempts.insert(
        1,
        AttemptObservation {
            alias: "acct".into(),
            group: g("g"),
            phase: AdmissionPhase::Waiting { since_ms: 0 },
            elapsed_ms,
        },
    );
    activity
}

#[test]
fn projection_is_bounded_and_carries_cooldowns_and_counters() {
    let mut activity = waiting_activity(9, 12_400);
    activity.admitted = 2;
    activity.hidden = 70;
    activity.completed = 5;
    activity.refused = 1;
    activity.cancelled = 2;
    activity.abandoned = 3;
    activity.groups.insert(
        g("cool"),
        GroupActivity {
            cooldown: Some(CooldownState::Until { until_ms: 40_500 }),
            last_refusal: Some("too many".into()),
        },
    );
    activity.groups.insert(
        g("gone"),
        GroupActivity {
            cooldown: Some(CooldownState::Unavailable),
            last_refusal: None,
        },
    );
    activity.groups.insert(
        g("vague"),
        GroupActivity {
            cooldown: Some(CooldownState::Unknown { since_ms: 1 }),
            last_refusal: None,
        },
    );
    let snapshot = project(&activity);
    assert_eq!(snapshot.waiting, 1);
    assert_eq!(snapshot.admitted, 2);
    assert_eq!(snapshot.longest_wait_seconds, Some(12));
    assert_eq!(snapshot.hidden, 70);
    assert_eq!(snapshot.revision, 9);
    assert_eq!(snapshot.counters.completed, 5);
    assert_eq!(snapshot.counters.refused, 1);
    assert_eq!(snapshot.counters.cancelled, 2);
    assert_eq!(snapshot.counters.abandoned, 3);
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["groups"][0]["group"], "cool");
    assert_eq!(json["groups"][0]["cooldown"]["state"], "until");
    assert_eq!(json["groups"][0]["cooldown"]["remainingSeconds"], 30);
    assert_eq!(json["groups"][0]["lastRefusal"], "too many");
    assert_eq!(json["groups"][1]["group"], "g");
    assert!(json["groups"][1].get("cooldown").is_none());
    assert_eq!(json["groups"][2]["cooldown"]["state"], "unavailable");
    assert!(
        json["groups"][2]["cooldown"]
            .get("remainingSeconds")
            .is_none()
    );
    assert_eq!(json["groups"][3]["cooldown"]["state"], "unknown");
    assert_eq!(json["longestWaitSeconds"], 12);
    let idle = project(&AdmissionActivity::default());
    assert_eq!(idle.longest_wait_seconds, None);
    assert!(
        serde_json::to_value(&idle)
            .unwrap()
            .get("longestWaitSeconds")
            .is_none()
    );
}

#[test]
fn waiting_progress_names_count_group_wait_and_cause() {
    assert_eq!(waiting_progress(&AdmissionActivity::default()), None);
    let mut activity = waiting_activity(1, 3_900);
    let (state, reason) = waiting_progress(&activity).unwrap();
    assert_eq!(state, "waiting");
    assert_eq!(
        reason,
        "1 inference attempt waiting for admission in group g for 3s"
    );
    activity.waiting = 2;
    activity.groups.get_mut(&g("g")).unwrap().cooldown =
        Some(CooldownState::Until { until_ms: 25_000 });
    let (_, reason) = waiting_progress(&activity).unwrap();
    assert_eq!(
        reason,
        "2 inference attempts waiting for admission in group g for 3s; cooldown 15s remaining"
    );
    activity.groups.get_mut(&g("g")).unwrap().cooldown =
        Some(CooldownState::Unknown { since_ms: 0 });
    assert!(
        waiting_progress(&activity)
            .unwrap()
            .1
            .ends_with("; throttled without a deadline")
    );
    activity.groups.get_mut(&g("g")).unwrap().cooldown = Some(CooldownState::Unavailable);
    assert!(
        waiting_progress(&activity)
            .unwrap()
            .1
            .ends_with("; group marked unavailable")
    );
}

#[test]
fn a_waiting_attempt_is_reported_as_waiting_never_quiet_or_active() {
    let source = Arc::new(ScriptedObservation::default());
    let mut state = ExecutionState::default();
    state.set_admission_source(source.clone());
    // Idle process, nothing queued: the usual quiet verdict.
    let quiet = state.snapshot();
    assert_eq!(quiet.progress.state, "quiet");
    assert_eq!(quiet.admission.as_ref().unwrap().waiting, 0);
    // A run whose first attempt is queued: "thinking" phase, waiting verdict.
    state.start_run();
    source.set(waiting_activity(1, 2_000));
    let waiting = state.snapshot();
    assert_eq!(waiting.phase, "thinking", "phase is untouched");
    assert_eq!(waiting.progress.state, "waiting");
    assert!(waiting.progress.reason.contains("group g"));
    assert_eq!(waiting.admission.as_ref().unwrap().waiting, 1);
    // Recent tool completions do not out-rank a queued attempt.
    state.observe(&crate::domain::agent::AgentProgressEvent::ToolStarted {
        tool_call_id: "c1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    });
    state.observe(&crate::domain::agent::AgentProgressEvent::ToolFinished {
        tool_call_id: "c1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
        result_content: String::new(),
        duration_ms: 1,
        is_error: false,
    });
    assert_eq!(state.snapshot().progress.state, "waiting");
    // The grant restores the evidence-based tool verdict.
    let mut admitted = AdmissionActivity {
        admitted: 1,
        revision: 2,
        ..AdmissionActivity::default()
    };
    admitted.groups.insert(g("g"), GroupActivity::default());
    source.set(admitted);
    let advancing = state.snapshot();
    assert_eq!(advancing.progress.state, "advancing");
    assert_eq!(advancing.admission.as_ref().unwrap().admitted, 1);
}

#[test]
fn admission_transitions_advance_the_public_cursor_exactly_once_each() {
    let source = Arc::new(ScriptedObservation::default());
    let mut state = ExecutionState::default();
    let base = state.observe_visible_revisions(0, 0);
    state.set_admission_source(source.clone());
    // Revision 0 is the untouched recorder: nothing to fold.
    assert_eq!(state.observe_visible_revisions(0, 0), base);
    source.set(waiting_activity(1, 0));
    let queued = state.observe_visible_revisions(0, 0);
    assert_eq!(queued, base + 1);
    assert_eq!(
        state.observe_visible_revisions(0, 0),
        queued,
        "an unchanged revision is folded once"
    );
    source.set(waiting_activity(7, 0));
    assert_eq!(state.observe_visible_revisions(0, 0), queued + 1);
}

#[test]
fn slim_get_state_carries_admission_and_since_sees_transitions() {
    let source = Arc::new(ScriptedObservation::default());
    let mut execution = ExecutionState::default();
    execution.set_admission_source(source.clone());
    let mut session = super::protocol::SessionState {
        model: "m".into(),
        generation: 0,
        is_streaming: true,
        session_key: "k".into(),
        message_count: 0,
        pending_message_count: 0,
        max_context_tokens: 0,
        effort: None,
        effort_levels: vec![],
        workflow: None,
        execution: None,
        sync: 0,
        control_receipts: vec![],
        automatic_turns_suspended: false,
        repeated_failure_notifications: Default::default(),
    };
    let mut poll = |since: Option<u64>, execution: &mut ExecutionState| {
        session.generation = execution.observe_visible_revisions(0, 0);
        session.execution = Some(execution.snapshot());
        slim_state_response_data(&session, since)
    };
    let first = poll(None, &mut execution);
    let generation = first["generation"].as_u64().unwrap();
    assert_eq!(first["admission"]["waiting"], 0);
    assert_eq!(first["state"], "idle", "admission is not a lifecycle state");
    assert_eq!(poll(Some(generation), &mut execution)["unchanged"], true);
    source.set(waiting_activity(1, 5_000));
    let changed = poll(Some(generation), &mut execution);
    assert!(changed.get("unchanged").is_none(), "{changed}");
    assert_eq!(changed["admission"]["waiting"], 1);
    assert_eq!(changed["admission"]["longestWaitSeconds"], 5);
    assert_eq!(changed["progress"]["state"], "waiting");
    // Without an admission source the field is absent altogether.
    let plain = slim_state_projection(&super::protocol::SessionState {
        execution: Some(ExecutionState::default().snapshot()),
        ..session.clone()
    });
    assert!(plain.get("admission").is_none());
}

#[test]
fn the_hook_broadcasts_one_admission_state_changed_event_per_transition() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);
    let hook = admission_event_hook(tx.clone());
    hook(&waiting_activity(3, 1_500));
    let line = rx.try_recv().unwrap();
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["type"], "admission_state_changed");
    assert_eq!(event["admission"]["waiting"], 1);
    assert_eq!(event["admission"]["revision"], 3);
    assert_eq!(event["admission"]["longestWaitSeconds"], 1);
    assert!(rx.try_recv().is_err(), "exactly one event");
    // No connected client: the transition is dropped, never an error.
    drop(rx);
    hook(&AdmissionActivity::default());
}

#[test]
fn the_probe_polls_the_slim_projection_like_a_supervisor() {
    let source = Arc::new(ScriptedObservation::default());
    let mut probe = super::AdmissionStateProbe::new(source.clone());
    let first = probe.poll(None);
    assert_eq!(first["state"], "idle");
    let generation = first["generation"].as_u64().unwrap();
    probe.start_run();
    source.set(waiting_activity(1, 1_000));
    let waiting = probe.poll(Some(generation));
    assert_eq!(waiting["state"], "thinking");
    assert_eq!(waiting["progress"]["state"], "waiting");
    probe.finish_run();
    source.set(AdmissionActivity {
        revision: 2,
        ..AdmissionActivity::default()
    });
    let done = probe.poll(None);
    assert_eq!(done["state"], "idle");
    assert_eq!(done["progress"]["state"], "quiet");
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(1);
    super::admission_broadcast_hook(tx)(&waiting_activity(4, 0));
    assert!(rx.try_recv().unwrap().contains("admission_state_changed"));
}

#[test]
fn execution_state_debug_names_the_phase_and_admission_presence() {
    let mut state = ExecutionState::default();
    assert!(format!("{state:?}").contains("admission: false"));
    state.set_admission_source(Arc::new(ScriptedObservation::default()));
    let text = format!("{state:?}");
    assert!(text.contains("admission: true") && text.contains("phase: \"idle\""));
}
