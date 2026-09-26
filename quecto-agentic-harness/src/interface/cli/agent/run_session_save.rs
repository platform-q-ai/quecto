//! Saving a one-shot run's transcript: after it completes, and after
//! `--max-time` stopped it (#2173).
use super::AgentOutput;
use crate::application::sessions::dto::SaveTrigger;
use crate::domain::message::Message;
use crate::interface::cli::uds_session_handles::SessionHandles;

pub(super) struct TranscriptSave<'a> {
    rt: &'a tokio::runtime::Runtime,
    sessions: &'a SessionHandles,
    /// The call-time system prompt, which is never persisted.
    system_prompt_id: Option<uuid::Uuid>,
}

impl<'a> TranscriptSave<'a> {
    pub(super) fn new(
        rt: &'a tokio::runtime::Runtime,
        sessions: &'a SessionHandles,
        system_prompt_id: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            rt,
            sessions,
            system_prompt_id,
        }
    }

    /// What a stopped run did stays on record: its files and sub-agents
    /// exist, so the transcript is saved with each unfinished call
    /// answered as stopped.
    pub(super) fn stopped(
        &self,
        messages: &mut Vec<Message>,
        secs: u64,
        out: &mut AgentOutput<'_>,
    ) {
        crate::domain::session_stopped::answer_unfinished_tool_calls(
            messages,
            &format!("max-time {secs}s stopped the run"),
        );
        self.save(messages, out);
    }

    pub(super) fn save(&self, messages: &mut Vec<Message>, out: &mut AgentOutput<'_>) {
        // Identity-based removal: immune to index shifts from mid-run
        // pruning (a no-op if pruning dropped it).
        if let Some(id) = self.system_prompt_id
            && let Some(idx) = messages.iter().position(|m| m.id() == id)
        {
            messages.remove(idx);
        }
        if let Err(e) = self.rt.block_on(
            self.sessions
                .save_session
                .save(messages, SaveTrigger::OrdinaryExit),
        ) {
            out.stderr
                .push_str(&format!("warning: failed to save session: {}\n", e));
        }
    }
}
