//! What opened the turn a message belongs to (#2226), and which reply is an
//! agent's report.
//!
//! A workflow engine drives a child between instructions with progress
//! nudges (auto-continue and template selection). The replies of those
//! turns are progress, not the answer to what the child was asked: they
//! must never replace a report the child already gave.
//!
//! Every message carries its turn's origin, stamped when it is appended
//! and persisted with it, so neither pruning nor a later reader's page
//! boundary can change it:
//! - a turn is an instruction turn only when it opens on a known
//!   instruction: a prompt, a task (`follow_up`), a steer, a queued
//!   control, or the completion nudge (which asks for the report);
//! - an auto-continue or template-selection nudge opens a progress turn;
//! - a harness note (a sub-agent's completion note, a swarm wake) opens a
//!   turn of the phase it lands in: after an instruction it continues that
//!   instruction's work (a child that waited for its reviewer answers
//!   then), during the nudge phase it is progress too;
//! - everything the loop appends inside a turn (replies, tool results, its
//!   own feedback) inherits the turn's origin;
//! - a message never stamped (saved before #2226, or of a turn still
//!   running) is of unknown origin.
//!
//! The report is the latest substantive reply of the best-ranked origin
//! present: an answer to an instruction, else an unmarked reply (a session
//! saved before #2226 answered its task unmarked), else a nudge reply. So a
//! nudge turn's reply is never the report while an answer exists, even when
//! the nudge caused real work: the workflow asks for its own report when it
//! completes (the completion nudge opens an instruction turn, so its reply
//! is the report), and a workflow that stops short leaves the last answer
//! as the report, with the nudge turns still readable after it. A report is
//! never withheld: an agent that only ever replied to nudges reports that.
use super::message::{Message, Role};

/// What opened the turn a message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnOrigin {
    /// Unmarked: saved before #2226, or of a turn still running.
    #[default]
    Unknown,
    /// A known instruction: something to answer.
    Instruction,
    /// A workflow progress nudge the engine injected.
    ProgressNudge,
    /// Marked with an origin this build does not know (a newer build's):
    /// it may be anything, so it ranks below every origin this build knows.
    Unrecognised,
}

impl TurnOrigin {
    /// How strongly a reply of this origin claims to be the report: an
    /// answer to an instruction, then an unmarked reply, then a nudge's,
    /// then one of an origin this build does not know.
    pub fn report_rank(self) -> u8 {
        match self {
            Self::Instruction => 3,
            Self::Unknown => 2,
            Self::ProgressNudge => 1,
            Self::Unrecognised => 0,
        }
    }

    /// Whether a reply of this origin may end the search for a report: an
    /// answer to an instruction, or an unmarked (legacy) one.
    pub fn answers(self) -> bool {
        matches!(self, Self::Instruction | Self::Unknown)
    }

    /// Whether the origin was stamped by this build's rules.
    pub fn is_known(self) -> bool {
        matches!(self, Self::Instruction | Self::ProgressNudge)
    }

    /// Whether the message was stamped at all: by this build's rules or a
    /// newer build's.
    pub fn is_stamped(self) -> bool {
        matches!(
            self,
            Self::Instruction | Self::ProgressNudge | Self::Unrecognised
        )
    }
}

/// A user message of `origin` that opens a turn.
fn opener(text: String, origin: TurnOrigin) -> Message {
    let mut message = Message::user(text);
    message.turn_origin = origin;
    message
}

/// A known instruction: a prompt, a task, a steer, a queued control, or
/// the completion nudge's request for the report.
pub fn instruction(text: String) -> Message {
    opener(text, TurnOrigin::Instruction)
}

/// A prompt (#2403): an instruction the user sent, or a task a parent or a
/// swarm gave; the watermark context pins the latest one.
pub fn prompt(text: String) -> Message {
    let mut message = instruction(text);
    message.user_kind = crate::domain::conversation::UserKind::Prompt;
    message
}

/// A progress nudge the workflow engine injects.
pub fn progress_nudge(text: String) -> Message {
    opener(text, TurnOrigin::ProgressNudge)
}

/// A harness note (a sub-agent's note, a swarm wake): it opens a turn of
/// the phase it lands in, [`current_phase`] of `messages`.
pub fn harness_note(text: String, messages: &[Message]) -> Message {
    opener(text, current_phase(messages))
}

/// The origin of the turn open at the end of `messages`: that of its latest
/// stamped message, unknown when none is.
pub fn current_phase(messages: &[Message]) -> TurnOrigin {
    messages
        .iter()
        .rev()
        .map(|message| message.turn_origin)
        .find(|origin| origin.is_known())
        .unwrap_or_default()
}

/// Stamp the turn `opener_id` opened: every message after the opener still
/// unstamped takes the opener's origin. An opener no longer in `messages`
/// stamps nothing, so its messages stay unknown rather than take another
/// turn's origin. Returns how many were stamped.
pub fn stamp_turn(messages: &mut [Message], opener_id: uuid::Uuid) -> usize {
    let Some(start) = messages.iter().position(|m| m.id() == opener_id) else {
        return 0;
    };
    let origin = messages[start].turn_origin;
    let mut stamped = 0;
    for message in &mut messages[start + 1..] {
        if message.turn_origin == TurnOrigin::Unknown {
            message.turn_origin = origin;
            stamped += 1;
        }
    }
    stamped
}

/// An assistant reply with non-blank text and no tool calls: a candidate
/// for the report.
pub fn is_substantive_reply(message: &Message) -> bool {
    message.role == Role::Assistant
        && message.tool_calls.is_empty()
        && !message.content.trim().is_empty()
}

/// The index of the report among `len` candidates, each described by
/// index: the latest substantive reply of the best [`TurnOrigin::report_rank`]
/// present, so an agent that only ever replied to nudges still reports.
pub fn report_index(
    len: usize,
    substantive: impl Fn(usize) -> bool,
    origin: impl Fn(usize) -> TurnOrigin,
) -> Option<usize> {
    let best = (0..len)
        .filter(|&i| substantive(i))
        .map(|i| origin(i).report_rank())
        .max()?;
    (0..len)
        .rev()
        .find(|&i| substantive(i) && origin(i).report_rank() == best)
}

/// The report a context ceiling keeps from removal (#2226): the report of
/// the finished turns, and only when it can be stubbed and recalled: when
/// it was spilled (a stub always was: pruning stubs only spilled messages).
/// A report that was never spilled can only be removed, and keeping it
/// could hold the context over its ceiling for good, so it is not kept.
///
/// The turn in flight is left out (see [`turn_in_flight_start`]): its replies
/// are not yet stamped, so an unmarked one would outrank a finished nudge
/// reply. A harness note opens a turn like any other opener, so a note
/// between the task and the answer never hides the answer (#2246).
///
/// Limitation: a session saved before #2226 marks nothing, so its answer
/// and the nudge replies after it rank alike, and the latest of them is
/// kept (no effort is spent on such sessions).
pub fn report_to_keep(messages: &[Message]) -> Option<usize> {
    let finished = turn_in_flight_start(messages);
    let report = transcript_report_index(&messages[..finished], is_substantive_reply)?;
    debug_assert!(report < finished);
    messages[report].spill_id.is_some().then_some(report)
}

/// A message that opens a turn: a user message the loop did not append
/// inside a turn (a prompt, a task, a nudge, a note, a steer). A watermark
/// cut's stub opens none (#2403).
pub fn opens_turn(message: &Message) -> bool {
    use crate::domain::conversation::UserKind;
    let opener_kind = match message.user_kind {
        UserKind::Prompt | UserKind::Unmarked => true,
        UserKind::ArchiveStub => false,
    };
    message.role == Role::User && message.turn.is_none() && opener_kind
}

/// The index of the latest message of `messages` that opens a turn: the
/// prompt a context region starts from.
pub fn latest_opener(messages: &[Message]) -> Option<usize> {
    let opener = messages.iter().rposition(opens_turn);
    debug_assert!(opener.is_none_or(|index| opens_turn(&messages[index])));
    opener
}

/// Where the turn in flight starts: at the latest opener while its turn is
/// still running, the end of `messages` when every turn is finished.
///
/// A turn is finished once [`stamp_turn`] stamped it at its end: its opener
/// carries a stamp and so does every message after it. An opener with
/// nothing after it has produced no reply yet, so nothing is left out. An
/// unmarked opener (a note in a session saved before #2226) cannot say, so
/// its turn is taken to be running. A resumed transcript pruned before its
/// next prompt is thus finished throughout (#2246).
fn turn_in_flight_start(messages: &[Message]) -> usize {
    let Some(opener) = latest_opener(messages) else {
        return messages.len();
    };
    let finished = messages[opener].turn_origin.is_stamped()
        && messages[opener + 1..]
            .iter()
            .all(|message| message.turn_origin.is_stamped());
    let start = if finished { messages.len() } else { opener };
    debug_assert!(start <= messages.len());
    start
}

/// What a page says of the report it names: never its text (#2226).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRef {
    pub id: String,
    pub ordinal: Option<u64>,
    pub origin: TurnOrigin,
    pub content_length: usize,
}

impl ReportRef {
    pub fn of(message: &Message) -> Self {
        Self {
            id: message.id().to_string(),
            ordinal: message.ordinal,
            origin: message.turn_origin,
            content_length: message.content.len(),
        }
    }
}

/// The report of a transcript: [`report_index`] over its messages' own
/// origins, with `substantive` deciding the candidates.
pub fn transcript_report_index(
    messages: &[Message],
    substantive: impl Fn(&Message) -> bool,
) -> Option<usize> {
    report_index(
        messages.len(),
        |i| substantive(&messages[i]),
        |i| messages[i].turn_origin,
    )
}

#[cfg(test)]
#[path = "turn_origin_tests.rs"]
mod tests;
