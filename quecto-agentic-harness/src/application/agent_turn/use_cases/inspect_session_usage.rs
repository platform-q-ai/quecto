//! Inspect token usage and cost (#1855): a session's usage, retained
//! across its turns. Each turn's billed usage (already costed at the active
//! model's catalogue price, the one price authority) and its request
//! diagnostics are accumulated here; a history replacement clears them; the
//! report pairs them with the history's message counts and the context
//! occupancy for `get_session_stats`.
use crate::application::agent_turn::agent_loop::UsageTotals;
use crate::application::agent_turn::dto::session_usage::{
    MessageCounts, SessionUsage, SessionUsageReport,
};
use crate::domain::conversation::value_objects::message::{Message, Role};
use crate::domain::inference::events::request_observation::RequestDiagnostics;

/// One session's retained usage.
#[derive(Debug, Default)]
pub struct InspectSessionUsage {
    usage: SessionUsage,
}

impl InspectSessionUsage {
    /// A session that has used nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulate one turn's billed usage. `session_key` and
    /// `context_tokens` name the session and its occupancy in the
    /// normalized usage log (#1567), written whenever the turn billed
    /// anything.
    pub fn record_turn(&mut self, session_key: &str, turn: UsageTotals, context_tokens: usize) {
        let tokens = &mut self.usage.tokens;
        tokens.input = tokens.input.saturating_add(turn.billed_input_tokens);
        tokens.output = tokens.output.saturating_add(turn.billed_output_tokens);
        tokens.cache_read = tokens.cache_read.saturating_add(turn.cache_read_tokens);
        tokens.cache_write = tokens.cache_write.saturating_add(turn.cache_write_tokens);
        tokens.total = tokens.input.saturating_add(tokens.output);
        self.usage.cost_micro_usd = self
            .usage
            .cost_micro_usd
            .saturating_add(turn.cost_micro_usd);
        self.usage.unpriced_attempts = self
            .usage
            .unpriced_attempts
            .saturating_add(turn.unpriced_attempts);
        let billed = [
            turn.billed_input_tokens,
            turn.billed_output_tokens,
            turn.cache_read_tokens,
            turn.cache_write_tokens,
            turn.cost_micro_usd,
            turn.unpriced_attempts,
        ];
        if billed.iter().any(|amount| *amount > 0) {
            let usage = &self.usage;
            tracing::info!(
                target: "session_usage",
                session_key = %session_key,
                input = usage.tokens.input,
                output = usage.tokens.output,
                cacheRead = usage.tokens.cache_read,
                cacheWrite = usage.tokens.cache_write,
                total = usage.tokens.total,
                contextTokens = context_tokens,
                // Absent when unknown (tracing omits a `None`), never 0;
                // `costPriced` says which (#2635 review).
                costMicroUsd = usage.known_cost_micro_usd(),
                costPriced = usage.known_cost_micro_usd().is_some(),
                unpricedAttempts = usage.unpriced_attempts,
                cacheHitRatio = usage.cache_hit_ratio(),
                "normalized session usage recorded"
            );
        }
    }

    /// Merge one turn's request diagnostics.
    pub fn record_request_diagnostics(&mut self, diagnostics: RequestDiagnostics) {
        self.usage.request_diagnostics.merge(diagnostics);
    }

    /// The history was replaced (a clear, a rewind, a switch): the usage
    /// retained for it goes with it.
    pub fn clear(&mut self) {
        self.usage = SessionUsage::default();
    }

    /// The usage retained so far.
    pub fn usage(&self) -> &SessionUsage {
        &self.usage
    }

    /// The session's report over its visible `messages`.
    pub fn report(
        &self,
        session_key: &str,
        messages: &[Message],
        context_tokens: usize,
        max_context_tokens: usize,
    ) -> SessionUsageReport {
        report_of(
            session_key,
            messages,
            self.usage.clone(),
            context_tokens,
            max_context_tokens,
        )
    }

    /// The report of a session that has used nothing yet: its history only
    /// (a loop's first stats, before any turn).
    pub fn history_report(session_key: &str, messages: &[Message]) -> SessionUsageReport {
        report_of(session_key, messages, SessionUsage::default(), 0, 0)
    }
}

fn report_of(
    session_key: &str,
    messages: &[Message],
    usage: SessionUsage,
    context_tokens: usize,
    max_context_tokens: usize,
) -> SessionUsageReport {
    let mut counts = MessageCounts {
        total: messages.len(),
        ..MessageCounts::default()
    };
    for message in messages {
        match message.role {
            Role::User => counts.user += 1,
            Role::Assistant => {
                counts.assistant += 1;
                counts.tool_calls += message.tool_calls.len();
            }
            Role::Tool => counts.tool_results += 1,
            Role::System => {}
        }
    }
    SessionUsageReport {
        session_key: session_key.to_owned(),
        messages: counts,
        cost_micro_usd: usage.known_cost_micro_usd(),
        cost_usd: usage.cost_usd(),
        cache_hit_ratio: usage.cache_hit_ratio(),
        usage,
        context_tokens,
        max_context_tokens,
    }
}

#[cfg(test)]
#[path = "inspect_session_usage_tests.rs"]
mod tests;
