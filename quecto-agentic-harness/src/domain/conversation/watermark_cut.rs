//! The watermark cut over the conversation's messages (#2403): the
//! planner's view of each message, the stub a cut puts in place, the
//! archive index it writes, and the cut itself. Pure: the caller writes the
//! index to session memory and holds the state between cuts.

use super::watermark::{CutPlan, PlanMessage, PlanRole};
use crate::domain::message::Message;

/// How the planner sees `message`.
pub fn plan_role(message: &Message) -> PlanRole<'_> {
    let _ = message;
    PlanRole::System
}

/// The planner's view of `messages`, in order, with their estimates.
pub fn plan_messages(messages: &[Message]) -> Vec<PlanMessage<'_>> {
    let _ = messages;
    Vec::new()
}

/// The stub a cut puts in place of `archived` messages: it names the
/// archive index `index_id` when one was written, and says the messages
/// were dropped when none was.
pub fn archive_stub(archived: usize, index_id: Option<&str>) -> Message {
    let _ = (archived, index_id);
    Message::user(String::new())
}

/// The most estimated tokens a stub can take, whatever it names: the
/// planner plans with it before the stub's text is known.
pub fn stub_tokens_bound() -> usize {
    0
}

/// The messages `plan` archives, in order.
pub fn archived<'a>(messages: &'a [Message], plan: &CutPlan) -> Vec<&'a Message> {
    let _ = (messages, plan);
    Vec::new()
}

/// The archive index of `archived`: the previous stub first, then one
/// line per message with its recall id, or its content in full when it was
/// never retained.
pub fn archive_index(archived: &[&Message]) -> String {
    let _ = archived;
    String::new()
}

/// Make the cut `plan` describes: the archived messages leave `messages`
/// (returned, in order) and `stub` goes in before the kept tail; nothing
/// kept changes.
pub fn apply_cut(messages: &mut Vec<Message>, plan: &CutPlan, stub: Message) -> Vec<Message> {
    let _ = (messages, plan, stub);
    Vec::new()
}

#[cfg(test)]
#[path = "watermark_cut_tests.rs"]
mod tests;
