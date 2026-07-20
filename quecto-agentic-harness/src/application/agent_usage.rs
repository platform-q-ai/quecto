use crate::domain::message::UsageInfo;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct UsageTotals {
    pub context_input_tokens: u32,
    pub output_tokens: u32,
    pub billed_input_tokens: u64,
    pub billed_output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_micro_usd: u64,
}

impl UsageTotals {
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
        if let Some(cost) = &usage.cost {
            self.cost_micro_usd = self
                .cost_micro_usd
                .saturating_add(cost.total_cost_micro_usd);
        }
    }
}

#[cfg(test)]
#[path = "agent_usage_tests.rs"]
mod tests;
