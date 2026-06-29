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
            context_tokens: None,
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
            context_tokens: None,
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
    fn record_uses_normalized_context_tokens_over_prompt_tokens() {
        // Anthropic-style: prompt_tokens is the non-cached delta, but
        // context_tokens carries the true occupancy (delta + cache). The
        // context gauge must reflect the latter while billing stays on
        // prompt_tokens.
        let mut totals = UsageTotals::default();
        totals.record(&UsageInfo {
            prompt_tokens: 100,
            completion_tokens: 5,
            cache_read_tokens: Some(80),
            cache_write_tokens: Some(20),
            context_tokens: Some(200),
            cost: None,
        });
        assert_eq!(
            totals.context_input_tokens, 200,
            "gauge uses context_tokens"
        );
        assert_eq!(
            totals.billed_input_tokens, 100,
            "billing stays on prompt_tokens"
        );
    }

    #[test]
    fn record_falls_back_to_prompt_tokens_when_context_tokens_absent() {
        // OpenAI-style: context_tokens is None because prompt_tokens already
        // counts the full prompt.
        let mut totals = UsageTotals::default();
        totals.record(&UsageInfo {
            prompt_tokens: 150,
            completion_tokens: 5,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_tokens: None,
            cost: None,
        });
        assert_eq!(totals.context_input_tokens, 150);
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
            context_tokens: None,
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
