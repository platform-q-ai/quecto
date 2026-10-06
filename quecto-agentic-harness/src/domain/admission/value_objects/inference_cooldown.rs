//! Group-owned fallback cooldown arithmetic with caller-injected randomness.
use super::inference_admission::AdmissionError;

/// One instance per quota group; it schedules no retries and owns no clock/RNG.
#[derive(Debug)]
pub struct FallbackCooldown {
    base: u64,
    maximum: u64,
    next_upper: u64,
}

impl FallbackCooldown {
    pub fn new(base: u64, maximum: u64) -> Result<Self, AdmissionError> {
        if base == 0 || maximum < base {
            return Err(AdmissionError::InvalidConfig);
        }
        Ok(Self {
            base,
            maximum,
            next_upper: base,
        })
    }

    /// Map the full injected sample range into the inclusive exponential window.
    pub fn throttle(&mut self, jitter: u64) -> u64 {
        let upper = self.next_upper;
        self.next_upper = upper.saturating_mul(2).min(self.maximum);
        // Widen before multiplication. Since base is positive, even the widest
        // window has at most u64::MAX buckets and the product fits in u128.
        let width = u128::from(upper - self.base) + 1;
        let offset = (u128::from(jitter) * width) >> 64;
        self.base + offset as u64
    }

    /// Only confirmed success resets the consecutive-throttle window.
    pub fn success(&mut self) {
        self.next_upper = self.base;
    }
}
