//! The watermark context (SPIKE, spike/watermark-context).
//!
//! A provider's prompt cache matches the longest identical prefix of a
//! request. Today's pruning edits the middle of the context (late
//! collapses, snapshot supersession, the ladder), and every edit misses the
//! cache from that point on. The watermark context instead appends only,
//! until the context reaches the high mark; then it cuts once, down to at
//! most the low mark, keeping:
//!
//! - the head: the leading system messages and the first user message
//!   (the brief), unchanged;
//! - one archive stub right after the head, which names the archive entry
//!   that indexes everything the cut removed (byte-stable until the next
//!   cut);
//! - the most recent whole exchanges that fit.
//!
//! A cut happens only at an exchange boundary: before a user message or
//! before an assistant message. Tool results follow the assistant message
//! that called them, and its reasoning travels on it, so no call loses its
//! result and no reasoning item is orphaned.
//!
//! Pure policy over the message list: no store, no I/O.

use crate::domain::message::{Message, Role};

/// The two marks, in provider tokens of the whole request (system prompt,
/// tool definitions and messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextWatermark {
    high_tokens: usize,
    low_tokens: usize,
}

impl ContextWatermark {
    /// The marks when `0 < low < high`; `None` otherwise.
    pub fn new(high_tokens: usize, low_tokens: usize) -> Option<Self> {
        (low_tokens > 0 && low_tokens < high_tokens).then_some(Self {
            high_tokens,
            low_tokens,
        })
    }

    pub fn high_tokens(&self) -> usize {
        self.high_tokens
    }

    pub fn low_tokens(&self) -> usize {
        self.low_tokens
    }
}

/// Room left for the archive stub when the low mark is planned: a stub is
/// about 60 estimated tokens; the reserve keeps the kept set under the
/// mark once the stub is in.
pub const STUB_RESERVE_TOKENS: usize = 128;

/// Where one cut falls: `messages[..head_end]` is the head,
/// `messages[head_end..tail_start]` is archived (a previous stub with
/// it), and `messages[tail_start..]` is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatermarkCut {
    pub head_end: usize,
    pub tail_start: usize,
    /// The kept set is still over the low mark: even the newest exchange
    /// alone does not fit, and it is kept whole regardless.
    pub over_low: bool,
}

/// Whether `msg` is an archive stub a previous cut placed: a pinned user
/// message (nothing else pins a user message).
pub fn is_archive_stub(msg: &Message) -> bool {
    msg.role == Role::User && msg.is_pinned
}

/// The end of the head: just past the first user message, when the
/// conversation opens with system messages and then a user message.
fn head_end(messages: &[Message]) -> Option<usize> {
    let first = messages.iter().position(|m| m.role != Role::System)?;
    match messages[first].role {
        Role::User if !is_archive_stub(&messages[first]) => Some(first + 1),
        Role::User | Role::Assistant | Role::Tool | Role::System => None,
    }
}

/// Whether a cut may fall just before `messages[i]`: a user message (not a
/// stub) or an assistant message opens an exchange.
fn is_boundary(msg: &Message) -> bool {
    match msg.role {
        Role::User => !is_archive_stub(msg),
        Role::Assistant => true,
        Role::Tool | Role::System => false,
    }
}

/// The cut to make before the next request, if any. `high` and `low` are
/// the marks for the messages alone, in estimate units (the caller takes
/// off the tool definitions and applies the provider-observed scale).
///
/// No cut below `high`; none when the conversation has no head, or when
/// no boundary would archive anything besides a previous stub.
pub fn plan_cut(messages: &[Message], high: usize, low: usize) -> Option<WatermarkCut> {
    let tokens: Vec<usize> = messages.iter().map(Message::estimated_tokens).collect();
    let total: usize = tokens.iter().sum();
    if total < high {
        return None;
    }
    let head_end = head_end(messages)?;
    // A previous stub is archived with the rest; archiving only it is no cut.
    let first_archived_real = match messages.get(head_end) {
        Some(msg) if is_archive_stub(msg) => head_end + 1,
        Some(_) | None => head_end,
    };
    let head_tokens: usize = tokens[..head_end].iter().sum();
    let budget = low.saturating_sub(head_tokens + STUB_RESERVE_TOKENS);
    // Walk back from the newest message: the earliest boundary whose
    // suffix fits is the cut; the newest boundary is kept whatever it costs.
    let mut suffix = 0usize;
    let mut fitting: Option<usize> = None;
    let mut newest: Option<usize> = None;
    for i in (first_archived_real + 1..messages.len()).rev() {
        suffix += tokens[i];
        if !is_boundary(&messages[i]) {
            continue;
        }
        newest.get_or_insert(i);
        if suffix <= budget {
            fitting = Some(i);
        } else {
            break;
        }
    }
    let tail_start = fitting.or(newest)?;
    debug_assert!(tail_start > first_archived_real, "a cut archives something");
    debug_assert!(
        is_boundary(&messages[tail_start]),
        "a cut falls on a boundary"
    );
    Some(WatermarkCut {
        head_end,
        tail_start,
        over_low: fitting.is_none(),
    })
}

/// Make `cut`: the archived messages leave `messages` (returned, in order)
/// and `stub` takes their place right after the head.
pub fn apply_cut(messages: &mut Vec<Message>, cut: WatermarkCut, stub: Message) -> Vec<Message> {
    assert!(is_archive_stub(&stub), "the stub is a pinned user message");
    assert!(
        cut.head_end < cut.tail_start && cut.tail_start <= messages.len(),
        "a cut lies inside the conversation: {cut:?} of {}",
        messages.len()
    );
    let archived: Vec<Message> = messages
        .splice(cut.head_end..cut.tail_start, std::iter::once(stub))
        .collect();
    debug_assert!(tail_is_whole(&messages[cut.head_end + 1..]));
    archived
}

/// Every tool result kept has its call kept.
fn tail_is_whole(tail: &[Message]) -> bool {
    let calls: std::collections::BTreeSet<&str> = tail
        .iter()
        .flat_map(|m| &m.tool_calls)
        .map(|c| c.id.as_str())
        .collect();
    tail.iter()
        .filter_map(|m| m.tool_call_id.as_deref())
        .all(|id| calls.contains(id))
}

/// The archive stub: a pinned user message naming the archive entry.
/// `index_id` is `None` when the archive could not be written: the stub
/// then says the messages were dropped, and promises no recall.
pub fn archive_stub(archived: usize, index_id: Option<&str>) -> Message {
    let content = match index_id {
        Some(id) => format!(
            "[Context archive] {archived} earlier messages of this session were archived to keep \
             the context small. recall(\"{id}\") lists them with their recall ids; \
             recall(\"<id>\") reads any one in full."
        ),
        None => format!(
            "[Context archive] {archived} earlier messages of this session were dropped to keep \
             the context small; recall(\"list\") shows what session memory holds."
        ),
    };
    let mut stub = Message::user(content);
    stub.is_pinned = true;
    debug_assert!(is_archive_stub(&stub));
    stub
}

/// The archive entry's content: one line per archived message, with its
/// recall id when it was spilled, its content in full when it was not.
pub fn archive_index(archived: &[Message]) -> String {
    let mut index = String::new();
    for (n, msg) in archived.iter().enumerate() {
        let role = msg.role.as_str();
        let calls: Vec<String> = msg
            .tool_calls
            .iter()
            .map(|c| {
                let args = crate::domain::text::truncate_chars(&c.arguments, 80, 77, "...");
                format!("{}({args})", c.name)
            })
            .collect();
        let calls = if calls.is_empty() {
            String::new()
        } else {
            format!(" calls: {}", calls.join(", "))
        };
        let line = match msg.spill_id.as_deref() {
            Some(id) => {
                let preview = crate::domain::text::truncate_chars(&msg.content, 100, 97, "...")
                    .replace(['\n', '\r'], " ");
                format!("{n}. {role}: \"{preview}\"{calls} recall(\"{id}\")\n")
            }
            None => format!("{n}. {role}:{calls}\n{}\n", msg.content),
        };
        index.push_str(&line);
    }
    index
}

#[cfg(test)]
#[path = "watermark_tests.rs"]
mod tests;
