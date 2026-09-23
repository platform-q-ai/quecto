//! Agent Commander spike: how the loop reports to the dry-run observer.
use super::*;
use crate::application::agent_commander::ports::{CommanderEvent, CommanderSink};

/// The loop's optional observer.
pub(super) type CommanderHandle = Option<Arc<dyn CommanderSink>>;

impl AgentLoopImpl {
    /// Attach the dry-run observer (never acts).
    pub fn set_commander(&mut self, commander: Option<Arc<dyn CommanderSink>>) {
        self.commander = commander;
    }

    /// The attached observer, if any.
    pub fn commander(&self) -> Option<Arc<dyn CommanderSink>> {
        self.commander.clone()
    }

    /// Report an event (fire-and-forget).
    pub(super) fn commander_observe(&self, event: CommanderEvent) {
        if let Some(commander) = &self.commander {
            commander.observe(&self.session_key, &self.model, event);
        }
    }

    /// The text of the last user message that is not a tool result.
    fn commander_prompt(messages: &[Message]) -> String {
        messages
            .iter()
            .rev()
            .find(|m| m.role == crate::domain::message::Role::User && m.tool_call_id.is_none())
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }

    /// A final response ended the turn (#5, #6, #22).
    pub(super) fn commander_turn_end(
        &self,
        messages: &[Message],
        response: &LlmResponse,
        turn: u32,
        tool_rounds: u32,
    ) {
        self.commander_observe(CommanderEvent::TurnEnd {
            turn,
            prompt: Self::commander_prompt(messages),
            final_text: response.content.clone().unwrap_or_default(),
            stop_reason: response
                .stop_reason
                .as_ref()
                .map(|r| r.as_str().to_string()),
            tool_rounds,
            ended_by: "final_response".into(),
            output_tokens: response.usage.as_ref().map(|u| u.completion_tokens),
            max_tokens: self.max_tokens,
        });
    }

    /// The tool-round cap ended the turn (#5, #22).
    pub(super) fn commander_iteration_limit(
        &self,
        messages: &[Message],
        turn: u32,
        tool_rounds: u32,
    ) {
        self.commander_observe(CommanderEvent::TurnEnd {
            turn,
            prompt: Self::commander_prompt(messages),
            final_text: String::new(),
            stop_reason: None,
            tool_rounds,
            ended_by: "iteration_limit".into(),
            output_tokens: None,
            max_tokens: self.max_tokens,
        });
    }
}
