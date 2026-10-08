//! The watermark cut over the conversation's messages (#2403): the
//! planner's view of each message, the stub a cut puts in place, the
//! archive index it writes, and the cut itself. Pure: the caller writes the
//! index to session memory and holds the state between cuts.

use super::watermark::{CutPlan, PlanMessage, PlanRole};
use crate::domain::conversation::services::turn_origin::opens_turn;
use crate::domain::conversation::value_objects::message::{Message, Role};
use crate::domain::conversation::value_objects::user_kind::UserKind;
use crate::domain::text::truncate_chars;

/// How the planner sees `message`: a user message is a prompt or a stub
/// only when it was marked one; every other user message is the harness's,
/// an opener when it opens a turn (a note, a wake, a nudge), the loop's own
/// feedback otherwise. A session saved before #2403 carries no marks: its
/// prompts plan as the harness's (no legacy effort, the harness has no
/// users yet).
pub fn plan_role(message: &Message) -> PlanRole<'_> {
    match (&message.role, message.user_kind) {
        (Role::System, _) => PlanRole::System,
        (Role::User, UserKind::Prompt) => PlanRole::Prompt,
        (Role::User, UserKind::ArchiveStub) => PlanRole::ArchiveStub,
        (Role::User, UserKind::Unmarked) => match opens_turn(message) {
            true => PlanRole::Opener,
            false => PlanRole::User,
        },
        (Role::Assistant, _) => PlanRole::Assistant {
            calls: message.tool_calls.iter().map(|c| c.id.as_str()).collect(),
        },
        (Role::Tool, _) => PlanRole::ToolResult {
            call: message.tool_call_id.as_deref().unwrap_or_default(),
        },
    }
}

/// The planner's view of `messages`, in order, with their estimates.
pub fn plan_messages(messages: &[Message]) -> Vec<PlanMessage<'_>> {
    messages
        .iter()
        .map(|message| PlanMessage {
            role: plan_role(message),
            tokens: message.estimated_tokens(),
        })
        .collect()
}

/// The stub's text: `archived` messages and the index naming them, or
/// none when it could not be written.
fn stub_text(archived: usize, index_id: Option<&str>) -> String {
    match index_id {
        Some(id) => format!(
            "[Context archive] {archived} earlier messages of this session were archived to \
             keep the context small. recall(\"{id}\") lists them with their recall ids; \
             recall(\"<id>\") reads any one in full."
        ),
        None => format!(
            "[Context archive] {archived} earlier messages of this session were dropped to \
             keep the context small; recall(\"list\") shows what session memory still holds."
        ),
    }
}

/// The stub a cut puts in place of `archived` messages: it names the
/// archive index `index_id` when one was written, and is recalled by it
/// (so it is never retained again itself); it says the messages were
/// dropped when no index was written.
pub fn archive_stub(archived: usize, index_id: Option<&str>) -> Message {
    let mut stub = Message::user(stub_text(archived, index_id));
    stub.user_kind = UserKind::ArchiveStub;
    stub.spill_id = index_id.map(str::to_string);
    stub
}

/// The stub a rewind leaves in place of one whose archive it wiped
/// (#2403 review M2): it names no archive, and it is a new message, so the
/// storm guard's baseline (held by the old stub) is reset.
pub fn rewound_stub() -> Message {
    let mut stub = Message::user(
        "[Context archive] Earlier messages of this session were dropped to keep the context \
         small; a rewind cleared the archive that held them."
            .to_string(),
    );
    stub.user_kind = UserKind::ArchiveStub;
    stub
}

/// What `message` keeps once the session memory is wiped (a rewind): no
/// spill id, and a cut's stub becomes [`rewound_stub`] (#2403 review M2).
pub fn forget_retention(message: &mut Message) {
    match message.user_kind {
        UserKind::ArchiveStub => {
            // Same place, same durable ordinal: nothing new to a reader.
            let ordinal = message.ordinal;
            *message = rewound_stub();
            message.ordinal = ordinal;
        }
        UserKind::Prompt | UserKind::Unmarked => {}
    }
    message.spill_id = None;
}

/// The most estimated tokens a stub can take, whatever it names: the
/// planner plans with it before the stub's text is known. The longest
/// count and the longest index id (`archive:` and a `usize` suffix, as the
/// retention store numbers it) bound every stub.
pub fn stub_tokens_bound() -> usize {
    let longest = usize::MAX.to_string();
    let id = format!("archive:{longest}");
    let archived = Message::estimate_tokens(&stub_text(usize::MAX, Some(&id)));
    let dropped = Message::estimate_tokens(&stub_text(usize::MAX, None));
    archived.max(dropped)
}

/// The messages `plan` archives, in order.
pub fn archived<'a>(messages: &'a [Message], plan: &CutPlan) -> Vec<&'a Message> {
    plan.archived()
        .iter()
        .flat_map(|range| &messages[range.clone()])
        .collect()
}

/// The archive index of `archived`: the previous stub first, in full (it
/// names the archive before this one, so the archives chain), then one
/// line per message with its recall id, or its content in full when it was
/// never retained.
pub fn archive_index(archived: &[&Message]) -> String {
    let (stubs, messages): (Vec<&Message>, Vec<&Message>) =
        archived.iter().partition(|m| match m.user_kind {
            UserKind::ArchiveStub => true,
            UserKind::Prompt | UserKind::Unmarked => false,
        });
    let mut index = String::new();
    for stub in stubs {
        index.push_str(&format!("previous archive: {}\n", stub.content));
    }
    for (n, message) in messages.into_iter().enumerate() {
        index.push_str(&index_line(n + 1, message));
    }
    index
}

/// One message's line in the archive index.
fn index_line(n: usize, message: &Message) -> String {
    let role = match (&message.role, message.tool_name.as_deref()) {
        (Role::Tool, Some(tool)) => format!("tool {tool}"),
        (role, _) => role.as_str().to_string(),
    };
    let calls: Vec<String> = message
        .tool_calls
        .iter()
        .map(|c| {
            format!(
                "{}({})",
                c.name,
                truncate_chars(&c.arguments, 80, 77, "...")
            )
        })
        .collect();
    let calls = match calls.is_empty() {
        true => String::new(),
        false => format!(" calls: {}", calls.join(", ")),
    };
    match message.spill_id.as_deref() {
        Some(id) => {
            let preview =
                truncate_chars(&message.content, 100, 97, "...").replace(['\n', '\r'], " ");
            format!("{n}. {role}: \"{preview}\"{calls} recall(\"{id}\")\n")
        }
        None => format!("{n}. {role}:{calls}\n{}\n", message.content),
    }
}

/// Make the cut `plan` describes: the archived messages leave `messages`
/// (returned, in order) and `stub` goes in before the kept tail, numbered
/// as the newest message it replaces; nothing kept changes. `plan` must have been made for exactly these messages.
pub fn apply_cut(messages: &mut Vec<Message>, plan: &CutPlan, mut stub: Message) -> Vec<Message> {
    assert_eq!(stub.user_kind, UserKind::ArchiveStub, "the stub is marked");
    // The stub takes the durable ordinal of the newest message it replaces
    // (#2403 final review L2): a reader who read up to it sees nothing new,
    // and it falls between the head and the tail whenever the cut archived
    // anything after the head (otherwise no free ordinal exists there).
    stub.ordinal = archived(messages, plan)
        .iter()
        .filter_map(|m| m.ordinal)
        .max();
    let archived_count: usize = plan.archived().iter().map(|range| range.len()).sum();
    assert_eq!(
        plan.kept().len() + archived_count,
        messages.len(),
        "the plan is of these messages"
    );
    let tail_start = plan.kept()[plan.stub_slot()];
    let mut kept = vec![false; messages.len()];
    plan.kept().iter().for_each(|&i| kept[i] = true);
    let mut stub = Some(stub);
    let mut archived = Vec::with_capacity(archived_count);
    for (i, message) in std::mem::take(messages).into_iter().enumerate() {
        if i == tail_start {
            messages.extend(stub.take());
        }
        match kept[i] {
            true => messages.push(message),
            false => archived.push(message),
        }
    }
    assert!(stub.is_none(), "the stub went in before the tail");
    assert_eq!(archived.len(), archived_count, "every planned message left");
    archived
}

#[cfg(test)]
#[path = "watermark_cut_tests.rs"]
mod tests;
