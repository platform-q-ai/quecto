#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ContextGaugeCalibration {
    /// Provider-reported occupancy shown to users after the last exact LLM call,
    /// adjusted by estimated removals/demotions in subsequent pruning passes.
    reported_tokens: usize,
    /// Message-only estimate at the point represented by `reported_tokens`.
    estimated_tokens: usize,
    /// False until a provider supplies usage; without provider truth the gauge
    /// intentionally remains the internal estimate for providers that omit usage.
    has_provider_truth: bool,
}

impl ContextGaugeCalibration {
    pub(super) fn reconcile_before_call(&mut self, current_estimate: usize) -> usize {
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

    pub(super) fn observe_provider_truth(
        &mut self,
        reported_tokens: usize,
        estimate_at_call: usize,
    ) {
        self.reported_tokens = reported_tokens;
        self.estimated_tokens = estimate_at_call;
        self.has_provider_truth = true;
    }

    pub(super) fn observe_estimate_only(&mut self, estimate: usize) {
        if !self.has_provider_truth {
            self.reported_tokens = estimate;
            self.estimated_tokens = estimate;
        }
    }
}

#[cfg(test)]
mod tests {
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

        gauge.observe_estimate_only(10);
        assert_eq!(
            gauge.reconcile_before_call(130),
            1_030,
            "estimate-only observations must not replace provider truth once calibrated"
        );
    }
}
