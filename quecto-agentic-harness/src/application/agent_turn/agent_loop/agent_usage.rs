use crate::domain::conversation::value_objects::message::UsageInfo;

#[derive(Debug, Clone, Copy, Default)]
pub struct UsageTotals {
    pub context_input_tokens: u32,
    pub output_tokens: u32,
    pub billed_input_tokens: u64,
    pub billed_output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_micro_usd: u64,
    /// Provider attempts that reported usage with no price to cost it at
    /// (#1855), counted as `cost_micro_usd` is summed: every attempt that
    /// reported usage, a cut-short attempt included, not logical requests.
    /// Their usage counts, but their cost is unknown, never zero.
    pub unpriced_attempts: u64,
}

impl UsageTotals {
    /// Billed totals only (no context occupancy): the shape a caller records
    /// against a session tracker.
    pub fn billed(
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        cost_micro_usd: u64,
    ) -> Self {
        Self {
            billed_input_tokens: input_tokens,
            billed_output_tokens: output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            cost_micro_usd,
            ..Self::default()
        }
    }

    pub fn record(&mut self, usage: &UsageInfo) {
        self.context_input_tokens = usage.context_input_tokens();
        self.output_tokens = self.output_tokens.saturating_add(usage.completion_tokens);
        self.billed_input_tokens = self
            .billed_input_tokens
            .saturating_add(usage.prompt_tokens as u64);
        self.billed_output_tokens = self
            .billed_output_tokens
            .saturating_add(usage.completion_tokens as u64);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens.unwrap_or(0) as u64);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens.unwrap_or(0) as u64);
        let billed = usage.prompt_tokens > 0
            || usage.completion_tokens > 0
            || usage.cache_read_tokens.is_some_and(|tokens| tokens > 0)
            || usage.cache_write_tokens.is_some_and(|tokens| tokens > 0);
        match (&usage.cost, billed) {
            (Some(cost), _) => {
                self.cost_micro_usd = self
                    .cost_micro_usd
                    .saturating_add(cost.total_cost_micro_usd);
            }
            (None, true) => self.unpriced_attempts = self.unpriced_attempts.saturating_add(1),
            (None, false) => {}
        }
    }
}

#[cfg(test)]
#[path = "agent_usage_tests.rs"]
mod tests;
