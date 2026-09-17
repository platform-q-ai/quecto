//! Tests for the OAuth expiry margin applied on every credential write.

use super::*;

#[test]
fn test_expires_at_with_margin_subtracts_300_seconds() {
    let now = crate::infrastructure::time::unix_timestamp_secs();
    let result = expires_at_with_margin(3600);
    // Should be approximately now + 3600 - 300 = now + 3300
    let expected = now + 3300;
    assert!(
        (result - expected).abs() <= 2,
        "expected ~{}, got {} (diff: {})",
        expected,
        result,
        (result - expected).abs()
    );
}

#[test]
fn test_expires_at_with_margin_short_expiry() {
    let now = crate::infrastructure::time::unix_timestamp_secs();
    let result = expires_at_with_margin(600);
    let expected = now + 300;
    assert!(
        (result - expected).abs() <= 2,
        "expected ~{}, got {}",
        expected,
        result
    );
}

#[test]
fn test_expires_at_with_margin_zero_expiry() {
    let now = crate::infrastructure::time::unix_timestamp_secs();
    let result = expires_at_with_margin(0);
    // Should be now - 300 (already expired with margin)
    let expected = now - 300;
    assert!(
        (result - expected).abs() <= 2,
        "expected ~{}, got {}",
        expected,
        result
    );
}

// --- OAUTH_EXPIRY_MARGIN_SECS constant ---
#[test]
fn test_oauth_expiry_margin_is_five_minutes() {
    assert_eq!(OAUTH_EXPIRY_MARGIN_SECS, 300);
}
