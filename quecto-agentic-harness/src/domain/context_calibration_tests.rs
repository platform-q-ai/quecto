use super::*;
use crate::domain::message::Message;

#[test]
fn the_identity_scale_leaves_the_heuristic_unchanged() {
    let scale = EstimateScale::IDENTITY;
    assert_eq!(scale.permille(), 1_000);
    assert_eq!(scale.calibrated(12_345), 12_345);
    assert_eq!(scale.in_estimate_units(250_000), 250_000);
    assert_eq!(EstimateScale::default(), EstimateScale::IDENTITY);
}

/// #2212: the QA run's digit-heavy output (`seq k 100000`) is about 2
/// characters per token. The estimator counts it at that rate, so what the
/// provider ratio corrects is only the residual.
#[test]
fn digit_heavy_output_leaves_only_a_residual_for_the_ratio() {
    let digits: String = (1..=20_000).map(|n| format!("{n} ")).collect();
    let estimate = Message::estimate_tokens(&digits);
    // A tokeniser splitting digit runs at about 2 chars/token.
    let provider = digits.len().div_ceil(2);
    assert_eq!(
        estimate, provider,
        "the estimator counts digits at 2 chars/token"
    );
    assert_eq!(
        EstimateScale::observed(provider, estimate),
        EstimateScale::IDENTITY
    );
    // A tokeniser 10% denser still is corrected by the ratio.
    let scale = EstimateScale::observed(provider * 11 / 10, estimate);
    assert!((1_099..=1_101).contains(&scale.permille()));
    assert!(scale.calibrated(estimate) >= provider * 11 / 10);
}

#[test]
fn the_ratio_rounds_up_so_the_calibrated_figure_never_undercounts() {
    // 1001 / 1000 is 1.001 exactly; 1000 / 999 is 1.001001..., rounded up.
    assert_eq!(EstimateScale::observed(1_001, 1_000).permille(), 1_001);
    assert_eq!(EstimateScale::observed(1_000, 999).permille(), 1_002);
    let scale = EstimateScale::observed(1_000, 999);
    assert!(scale.calibrated(999) >= 1_000);
}

#[test]
fn the_ratio_never_scales_below_the_heuristic() {
    // A provider reporting less than the estimate (prose, cache quirks) keeps
    // the heuristic, which then errs towards pruning.
    assert_eq!(EstimateScale::observed(500, 1_000), EstimateScale::IDENTITY);
    assert_eq!(
        EstimateScale::observed(1_000, 1_000),
        EstimateScale::IDENTITY
    );
}

#[test]
fn the_ratio_is_capped_at_four() {
    assert_eq!(EstimateScale::observed(4_000, 1_000).permille(), 4_000);
    assert_eq!(EstimateScale::observed(4_001, 1_000).permille(), 4_000);
    let absurd = EstimateScale::observed(usize::MAX, 1);
    assert_eq!(absurd.permille(), EstimateScale::MAX_PERMILLE);
    assert_eq!(absurd.in_estimate_units(1_000), 250);
}

#[test]
fn an_observation_without_both_figures_is_the_identity() {
    // Zero estimate: no ratio exists (and no division by zero).
    assert_eq!(EstimateScale::observed(5_000, 0), EstimateScale::IDENTITY);
    // Zero report: the provider sent no usable usage.
    assert_eq!(EstimateScale::observed(0, 5_000), EstimateScale::IDENTITY);
    assert_eq!(EstimateScale::observed(0, 0), EstimateScale::IDENTITY);
}

#[test]
fn the_budget_in_estimate_units_rounds_down_towards_pruning() {
    let scale = EstimateScale::observed(3_000, 1_000);
    // 1000 / 3 = 333.33..., rounded down.
    assert_eq!(scale.in_estimate_units(1_000), 333);
    assert!(scale.calibrated(scale.in_estimate_units(1_000)) <= 1_000);
    assert_eq!(scale.in_estimate_units(0), 0);
}

#[test]
fn scaling_saturates_instead_of_overflowing() {
    let scale = EstimateScale::observed(4_000, 1_000);
    assert_eq!(scale.calibrated(usize::MAX), usize::MAX);
    assert_eq!(scale.in_estimate_units(usize::MAX), usize::MAX / 4);
}

#[test]
fn a_budget_that_fits_in_estimate_units_fits_in_calibrated_units() {
    for (reported, estimated) in [(1_000, 1_000), (2_017, 1_000), (7, 3), (39_999, 10_001)] {
        let scale = EstimateScale::observed(reported, estimated);
        for budget in [1, 17, 1_000, 250_000] {
            let units = scale.in_estimate_units(budget);
            assert!(
                scale.calibrated(units) <= budget,
                "{units} estimated tokens at {} permille exceed {budget}",
                scale.permille()
            );
        }
    }
}

/// Messages have the ceiling less the tool definitions when that is at
/// least a quarter of the ceiling (#2160).
#[test]
fn the_message_budget_is_the_room_the_tools_leave() {
    assert_eq!(
        message_budget(10_000, Some(10_000), 3_000),
        MessageBudget {
            tokens: 7_000,
            window_exceeded: false
        }
    );
    assert_eq!(message_budget(10_000, None, 3_000).tokens, 7_000);
}

/// Without a known window the quarter floor (#2182) stands: the ceiling is
/// the configured hot-context budget, not a provider limit.
#[test]
fn without_a_window_oversized_tools_leave_the_messages_a_quarter() {
    assert_eq!(
        message_budget(8_000, None, 7_000),
        MessageBudget {
            tokens: 2_000,
            window_exceeded: false
        }
    );
}

/// #2212 PR review, the reviewer's numbers: an 8,000-token window at a 2x
/// scale is a 4,000 estimate-unit ceiling; 3,500 of tools left the floor
/// granting 1,000, so 4,500 estimated (9,000 real) went to the provider.
#[test]
fn the_quarter_floor_never_grants_more_than_the_window_leaves() {
    let scale = EstimateScale::observed(2_000, 1_000);
    let window = scale.in_estimate_units(8_000);
    let budget = message_budget(window, Some(window), 3_500);
    assert_eq!(
        budget,
        MessageBudget {
            tokens: 500,
            window_exceeded: false
        }
    );
    assert!(scale.calibrated(3_500 + budget.tokens) <= 8_000);
}

/// The floor overshot at 1x too, before any calibration.
#[test]
fn the_window_guard_holds_without_calibration() {
    let budget = message_budget(8_000, Some(8_000), 7_000);
    assert_eq!(budget.tokens, 1_000);
    assert!(!budget.window_exceeded);
    // A configured ceiling under the window keeps its own floor when the
    // window has room for it.
    assert_eq!(message_budget(8_000, Some(100_000), 7_000).tokens, 2_000);
}

#[test]
fn tools_that_fill_the_window_are_reported_not_fitted() {
    for fixed in [4_000, 4_200, usize::MAX] {
        let budget = message_budget(4_000, Some(4_000), fixed);
        assert!(budget.window_exceeded, "{fixed}");
        assert_eq!(budget.tokens, 1_000, "the messages keep the quarter floor");
    }
}
