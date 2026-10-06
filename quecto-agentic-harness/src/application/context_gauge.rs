//! The provider-calibrated context gauge (split from `context.rs`, #2212).
//!
//! One provider observation (its reported prompt size against the local
//! estimate of the transcript it was sent) calibrates two things:
//!
//! - the user-facing gauge: the reported figure, carried forward by the
//!   estimate delta of later local changes until the next observation; and
//! - the [`EstimateScale`] the pruning ceiling decides with: the reported
//!   figure over the estimate, clamped (#2212).
//!
//! Staleness. Every observation replaces the scale outright (no averaging),
//! so it always describes the last transcript a provider counted. Pruning
//! runs before each request and an observation follows each response, so
//! the scale a pass uses was measured on the transcript the previous pass
//! left. A pass that stubs dense content leaves the old, higher scale in
//! place until the next response: it errs towards pruning, never away from
//! it. A model switch, a provider swap (another tokeniser) and a session
//! change (another transcript) forget both the scale and the provider
//! figure, so the next Thinking event shows no stale occupancy. Until the
//! next observation the estimate stands, as it does for a resumed session
//! and for providers that report no usage.

use crate::domain::inference::services::context_calibration::EstimateScale;

#[derive(Debug, Clone, Copy, Default)]
pub(in crate::application) struct ContextGaugeCalibration {
    /// Provider-reported occupancy shown to users after the last exact LLM call,
    /// adjusted by estimated removals/demotions in subsequent pruning passes.
    reported_tokens: usize,
    /// Message-only estimate at the point represented by `reported_tokens`.
    estimated_tokens: usize,
    /// False until a provider supplies usage; without provider truth the gauge
    /// intentionally remains the internal estimate for providers that omit usage.
    has_provider_truth: bool,
    /// The last observation's reported-over-estimated ratio (#2212); the
    /// identity until a provider reports usage and after it is forgotten.
    scale: EstimateScale,
}

impl ContextGaugeCalibration {
    pub(in crate::application) fn reconcile_before_call(
        &mut self,
        current_estimate: usize,
    ) -> usize {
        if self.has_provider_truth {
            if current_estimate < self.estimated_tokens {
                self.reported_tokens = self
                    .reported_tokens
                    .saturating_sub(self.estimated_tokens - current_estimate);
            } else if current_estimate > self.estimated_tokens {
                self.reported_tokens = self
                    .reported_tokens
                    .saturating_add(current_estimate - self.estimated_tokens);
            }
            self.estimated_tokens = current_estimate;
            self.reported_tokens
        } else {
            self.estimated_tokens = current_estimate;
            self.reported_tokens = current_estimate;
            current_estimate
        }
    }

    pub(in crate::application) fn observe_provider_truth(
        &mut self,
        reported_tokens: usize,
        estimate_at_call: usize,
    ) {
        self.reported_tokens = reported_tokens;
        self.estimated_tokens = estimate_at_call;
        self.has_provider_truth = true;
        self.scale = EstimateScale::observed(reported_tokens, estimate_at_call);
    }

    pub(in crate::application) fn observe_estimate_only(&mut self, estimate: usize) {
        if !self.has_provider_truth {
            self.reported_tokens = estimate;
            self.estimated_tokens = estimate;
        }
    }

    /// The scale the pruning ceiling decides with (#2212).
    pub(in crate::application) fn estimate_scale(&self) -> EstimateScale {
        self.scale
    }

    /// The last observation no longer describes what the next request
    /// sends (another session's transcript, another model's or provider's
    /// tokeniser): neither the provider figure nor the scale holds, so the
    /// gauge starts over and the estimate stands until the next report.
    pub(in crate::application) fn forget_calibration(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
#[path = "context_gauge_tests.rs"]
mod tests;
