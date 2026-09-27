use super::*;
use crate::domain::context_calibration::EstimateScale;

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

#[test]
fn the_scale_is_the_heuristic_until_a_provider_reports_usage() {
    let mut gauge = ContextGaugeCalibration::default();
    assert_eq!(gauge.estimate_scale(), EstimateScale::IDENTITY);

    gauge.observe_estimate_only(1_000);
    assert_eq!(
        gauge.estimate_scale(),
        EstimateScale::IDENTITY,
        "an estimate-only observation calibrates nothing"
    );

    gauge.observe_provider_truth(2_000, 1_000);
    assert_eq!(gauge.estimate_scale().permille(), 2_000);
}

/// #2212: each observation replaces the scale, so a pass that stubbed the
/// dense content is measured afresh by the next response.
#[test]
fn every_observation_replaces_the_scale() {
    let mut gauge = ContextGaugeCalibration::default();
    gauge.observe_provider_truth(4_000, 1_000);
    // Pruning changed the transcript; the old scale stays until the next
    // response (erring towards pruning) ...
    gauge.reconcile_before_call(400);
    assert_eq!(gauge.estimate_scale().permille(), 4_000);
    // ... which measures the pruned transcript on its own.
    gauge.observe_provider_truth(440, 400);
    assert_eq!(gauge.estimate_scale().permille(), 1_100);
    // A report below the estimate falls back to the heuristic.
    gauge.observe_provider_truth(300, 400);
    assert_eq!(gauge.estimate_scale(), EstimateScale::IDENTITY);
}

#[test]
fn a_forgotten_scale_is_the_heuristic_and_the_gauge_keeps_its_truth() {
    let mut gauge = ContextGaugeCalibration::default();
    gauge.observe_provider_truth(2_000, 1_000);

    gauge.forget_estimate_scale();

    assert_eq!(gauge.estimate_scale(), EstimateScale::IDENTITY);
    assert_eq!(
        gauge.reconcile_before_call(1_100),
        2_100,
        "the display gauge still carries the provider figure forward"
    );
    gauge.observe_provider_truth(3_000, 1_000);
    assert_eq!(gauge.estimate_scale().permille(), 3_000);
}

#[test]
fn a_forgotten_calibration_starts_the_gauge_over() {
    let mut gauge = ContextGaugeCalibration::default();
    gauge.observe_provider_truth(2_000, 1_000);

    gauge.forget_calibration();

    assert_eq!(gauge.estimate_scale(), EstimateScale::IDENTITY);
    assert_eq!(
        gauge.reconcile_before_call(1_100),
        1_100,
        "without provider truth the gauge is the estimate"
    );
}
