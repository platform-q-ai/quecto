use super::*;

#[test]
fn estimate_only_gauge_tracks_estimate_until_provider_truth_arrives() {
    let mut gauge = ContextGaugeCalibration::default();

    assert_eq!(gauge.reconcile_before_call(100), 100);
    gauge.observe_estimate_only(120);
    assert_eq!(gauge.reconcile_before_call(140), 140);
}

#[test]
fn provider_truth_is_carried_forward_by_estimate_delta() {
    let mut gauge = ContextGaugeCalibration::default();

    gauge.observe_provider_truth(1_000, 100);
    assert_eq!(gauge.reconcile_before_call(80), 980);
    assert_eq!(gauge.reconcile_before_call(130), 1_030);
    // Unchanged estimate keeps the calibrated provider value stable.
    assert_eq!(gauge.reconcile_before_call(130), 1_030);

    gauge.observe_estimate_only(10);
    assert_eq!(
        gauge.reconcile_before_call(130),
        1_030,
        "estimate-only observations must not replace provider truth once calibrated"
    );
}

#[test]
fn context_gauge_debug_fmt_includes_struct_name() {
    let gauge = ContextGaugeCalibration::default();
    let rendered = format!("{gauge:?}");
    assert!(
        rendered.contains("ContextGaugeCalibration"),
        "Debug output should name the calibration type: {rendered}"
    );
}
