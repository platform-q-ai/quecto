//! The lenient numbers of `search_session_metadata` (R1-H5). `limit` and
//! `generation` are decoded as any JSON value: a number is brought into
//! range — negative is 0, a fraction is truncated, anything beyond `u64` is
//! `u64::MAX` — and an absent or `null` field is absent. Anything else is
//! named, so the edge can refuse it in a CORRELATED answer: a strict decode
//! failure is an uncorrelated `parse_error`, and a client awaiting its id
//! would wait for ever.

/// The number in `value`: `Ok(None)` when absent, `Err(())` when not a number.
pub(super) fn lenient_u64(value: &serde_json::Value) -> Result<Option<u64>, ()> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => Ok(Some(match number.as_u64() {
            Some(exact) => exact,
            // Saturating: negative and NaN are 0, too large is `u64::MAX`.
            None => number.as_f64().map_or(0, |float| float as u64),
        })),
        _ => Err(()),
    }
}

#[cfg(test)]
#[path = "uds_search_numbers_tests.rs"]
mod tests;
