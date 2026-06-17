use crate::domain::message::UsageInfo;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct UsageTotals {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub cost: f64,
}

impl UsageTotals {
    pub fn record(&mut self, usage: &UsageInfo) {
        self.input_tokens = self.input_tokens.saturating_add(usage.prompt_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.completion_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens.unwrap_or(0));
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens.unwrap_or(0));
        if let Some(cost) = &usage.cost {
            self.cost += cost.total_cost_usd();
        }
    }
}
