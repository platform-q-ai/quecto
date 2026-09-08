//! #1679 P2 AC5: bounded group-wide fallback, not an additional retry owner.
use quecto::domain::inference_admission::AdmissionError;
use quecto::domain::inference_cooldown::FallbackCooldown;

#[test]
fn first_throttle_is_base_regardless_of_jitter() {
    let mut fallback = FallbackCooldown::new(10, 80).unwrap();
    assert_eq!(fallback.throttle(u64::MAX), 10);
}
#[test]
fn second_throttle_reaches_doubled_upper_bound() {
    let mut fallback = FallbackCooldown::new(10, 80).unwrap();
    fallback.throttle(0);
    assert_eq!(fallback.throttle(u64::MAX), 20);
}
#[test]
fn repeated_throttles_saturate_at_configured_maximum() {
    let mut fallback = FallbackCooldown::new(10, 80).unwrap();
    for _ in 0..100 {
        fallback.throttle(u64::MAX);
    }
    assert_eq!(fallback.throttle(u64::MAX), 80);
}
#[test]
fn lower_jitter_boundary_remains_base() {
    let mut fallback = FallbackCooldown::new(10, 80).unwrap();
    for _ in 0..100 {
        fallback.throttle(u64::MAX);
    }
    assert_eq!(fallback.throttle(0), 10);
}
#[test]
fn confirmed_success_resets_consecutive_count() {
    let mut fallback = FallbackCooldown::new(10, 80).unwrap();
    for _ in 0..4 {
        fallback.throttle(u64::MAX);
    }
    fallback.success();
    assert_eq!(fallback.throttle(u64::MAX), 10);
}
#[test]
fn zero_base_is_invalid() {
    assert!(matches!(
        FallbackCooldown::new(0, 80),
        Err(AdmissionError::InvalidConfig)
    ));
}
#[test]
fn maximum_smaller_than_base_is_invalid() {
    assert!(matches!(
        FallbackCooldown::new(81, 80),
        Err(AdmissionError::InvalidConfig)
    ));
}
#[test]
fn multiplication_saturates_without_overflow() {
    let mut fallback = FallbackCooldown::new(u64::MAX / 2 + 1, u64::MAX).unwrap();
    fallback.throttle(0);
    assert_eq!(fallback.throttle(u64::MAX), u64::MAX);
}
#[test]
fn equal_base_and_maximum_is_valid() {
    let mut fallback = FallbackCooldown::new(80, 80).unwrap();
    assert_eq!(fallback.throttle(u64::MAX), 80);
}
#[test]
fn middle_jitter_is_not_forced_to_an_endpoint() {
    let mut fallback = FallbackCooldown::new(10, 80).unwrap();
    fallback.throttle(0);
    assert_eq!(fallback.throttle(u64::MAX / 2), 15);
}

#[test]
fn independent_group_counter_is_not_advanced_by_another_group() {
    let mut first = FallbackCooldown::new(10, 80).unwrap();
    let mut second = FallbackCooldown::new(10, 80).unwrap();
    for _ in 0..4 {
        first.throttle(u64::MAX);
    }
    assert_eq!(second.throttle(u64::MAX), 10);
}
