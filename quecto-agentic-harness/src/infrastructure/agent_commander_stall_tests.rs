use super::*;

fn watch_calls(watch: &mut StallWatch, calls: &[(f64, bool)]) -> Vec<bool> {
    calls
        .iter()
        .map(|(ts, ok)| watch.record(*ts, "edit", *ok, "oldText not found"))
        .collect()
}

#[test]
fn six_failures_in_the_last_twelve_calls_trigger_one_stall_check() {
    let mut watch = StallWatch::default();
    let fired = watch_calls(
        &mut watch,
        &[
            (0.0, false),
            (10.0, true),
            (20.0, false),
            (30.0, false),
            (40.0, false),
            (50.0, false),
            (60.0, false),
        ],
    );
    assert_eq!(fired, [false, false, false, false, false, false, true]);
    assert_eq!(watch.window().len(), 7);
}

#[test]
fn normal_red_green_iteration_never_triggers() {
    let mut watch = StallWatch::default();
    let calls: Vec<(f64, bool)> = (0..30).map(|i| (i as f64 * 20.0, i % 3 != 0)).collect();
    assert!(watch_calls(&mut watch, &calls).iter().all(|fired| !fired));
}

#[test]
fn after_a_check_it_waits_for_six_more_failures() {
    let mut watch = StallWatch::default();
    let failures: Vec<(f64, bool)> = (0..12).map(|i| (i as f64, false)).collect();
    let fired = watch_calls(&mut watch, &failures);
    assert_eq!(fired.iter().filter(|f| **f).count(), 2, "{fired:?}");
    assert!(fired[5] && fired[11]);
}

#[test]
fn failures_spread_over_more_than_fifteen_minutes_do_not_trigger() {
    let mut watch = StallWatch::default();
    let slow: Vec<(f64, bool)> = (0..6).map(|i| (i as f64 * 400.0, false)).collect();
    assert!(watch_calls(&mut watch, &slow).iter().all(|fired| !fired));
}

#[test]
fn a_stuck_answer_maps_to_a_replacement_suggestion_for_the_supervisor() {
    let stuck = json!({"progress": {"choice": "stuck", "confidence": 0.8}});
    let child = AgentRole::Child { parent_id: None };
    assert_eq!(
        stall_action(&stuck, &child),
        Some("tell_parent_stalled_suggest_replacement")
    );
    assert_eq!(
        stall_action(&stuck, &AgentRole::Root),
        Some("tell_owner_stalled")
    );
    let unsure = json!({"progress": {"choice": "stuck", "confidence": 0.4}});
    assert_eq!(stall_action(&unsure, &child), Some("none_uncertain"));
    let fine = json!({"progress": {"choice": "making_progress", "confidence": 0.9}});
    assert_eq!(stall_action(&fine, &child), Some("none"));
    assert_eq!(stall_action(&json!({}), &child), None);
}
