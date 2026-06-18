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
        self.context_input_tokens = usage.prompt_tokens;
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
mod tests {
    use super::*;
    use crate::domain::message::CostInfo;

    #[test]
    fn usage_totals_record_accumulates_billed_usage_and_keeps_latest_context() {
        let mut totals = UsageTotals::default();
        totals.record(&UsageInfo {
            prompt_tokens: 10,
            completion_tokens: 2,
            cache_read_tokens: Some(3),
            cache_write_tokens: Some(4),
            cost: Some(CostInfo {
                input_cost_micro_usd: 0,
                output_cost_micro_usd: 0,
                cache_read_cost_micro_usd: 0,
                cache_write_cost_micro_usd: 0,
                total_cost_micro_usd: 1_000,
            }),
        });
        totals.record(&UsageInfo {
            prompt_tokens: 20,
            completion_tokens: 5,
            cache_read_tokens: None,
            cache_write_tokens: None,
            cost: None,
        });

        assert_eq!(totals.context_input_tokens, 20);
        assert_eq!(totals.output_tokens, 7);
        assert_eq!(totals.billed_input_tokens, 30);
        assert_eq!(totals.billed_output_tokens, 7);
        assert_eq!(totals.cache_read_tokens, 3);
        assert_eq!(totals.cache_write_tokens, 4);
        assert_eq!(totals.cost_micro_usd, 1_000);
    }

    #[test]
    fn usage_totals_record_saturates_counters() {
        let mut totals = UsageTotals {
            billed_input_tokens: u64::MAX,
            billed_output_tokens: u64::MAX,
            cache_read_tokens: u64::MAX,
            cache_write_tokens: u64::MAX,
            cost_micro_usd: u64::MAX,
            output_tokens: u32::MAX,
            context_input_tokens: 0,
        };
        totals.record(&UsageInfo {
            prompt_tokens: 1,
            completion_tokens: 1,
            cache_read_tokens: Some(1),
            cache_write_tokens: Some(1),
            cost: Some(CostInfo {
                input_cost_micro_usd: 0,
                output_cost_micro_usd: 0,
                cache_read_cost_micro_usd: 0,
                cache_write_cost_micro_usd: 0,
                total_cost_micro_usd: 1,
            }),
        });

        assert_eq!(totals.output_tokens, u32::MAX);
        assert_eq!(totals.billed_input_tokens, u64::MAX);
        assert_eq!(totals.billed_output_tokens, u64::MAX);
        assert_eq!(totals.cache_read_tokens, u64::MAX);
        assert_eq!(totals.cache_write_tokens, u64::MAX);
        assert_eq!(totals.cost_micro_usd, u64::MAX);
    }
}
