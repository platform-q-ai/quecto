//! The watermark cut planner (#2402).
//!
//! A prompt cache matches the longest identical prefix of a request, so the
//! watermark context only appends. When the request reaches the high mark
//! (H), one deep cut takes it down to the low mark (L). This module plans
//! that cut; it is pure: no I/O, no tokenizer. Token estimates come in, and
//! every token count here is in those estimate units.

use std::ops::Range;

/// The two marks, in estimated tokens of the whole request: messages plus
/// tool definitions. Always `0 < low < high`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watermark {
    high: usize,
    low: usize,
}

impl Watermark {
    /// The marks when `0 < low < high`; `None` otherwise.
    pub fn new(high: usize, low: usize) -> Option<Self> {
        (low > 0 && low < high).then_some(Self { high, low })
    }

    pub fn high(&self) -> usize {
        self.high
    }

    pub fn low(&self) -> usize {
        self.low
    }

    /// The effective marks under `ceiling`: H is `min(H, ceiling)`, and when
    /// the ceiling lowers H, L scales by the same ratio.
    pub fn under_ceiling(self, ceiling: usize) -> Self {
        match ceiling {
            at_or_above if at_or_above >= self.high => self,
            below => Self {
                high: below,
                low: scale(self.low, below, self.high),
            },
        }
    }
}

/// `value * numerator / denominator`, rounded down, for
/// `numerator < denominator`: never more than `value`.
fn scale(value: usize, numerator: usize, denominator: usize) -> usize {
    assert!(numerator < denominator, "a ceiling scales a mark down");
    let scaled = value as u128 * numerator as u128 / denominator as u128;
    usize::try_from(scaled).unwrap_or(value)
}

/// When a cut is due. It needs only the request's total, so a caller can
/// ask before it builds the planner's view of every message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutTrigger {
    pub marks: Watermark,
    /// The most the request may hold: the model window less the output
    /// reserve, or the configured maximum.
    pub ceiling: usize,
    /// The request's estimated tokens right after the previous cut, if one
    /// was made.
    pub after_last_cut: Option<usize>,
}

impl CutTrigger {
    /// The marks under the ceiling.
    pub fn effective_marks(&self) -> Watermark {
        self.marks.under_ceiling(self.ceiling)
    }

    /// Whether a request of `total` estimated tokens (messages plus tool
    /// definitions) reached H and, after a previous cut, has grown by at
    /// least `(H - L) / 2` since: one exchange over H never cuts every turn.
    pub fn is_due(&self, total: usize) -> bool {
        let marks = self.effective_marks();
        let grown = match self.after_last_cut {
            Some(after) => total >= after.saturating_add((marks.high - marks.low) / 2),
            None => true,
        };
        total >= marks.high && grown
    }
}

/// What the planner knows about one message: its role, and its links to
/// tool calls. Call ids are assumed unique; a result answers the latest
/// assistant message before it that made its call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRole<'a> {
    System,
    /// A user message the user sent: the prompt of a turn.
    Prompt,
    /// A user-role message the harness added: feedback, a sub-agent's
    /// note, a swarm wake-up.
    User,
    /// The stub a previous cut placed; it is archived by the next cut.
    ArchiveStub,
    /// An assistant message and the ids of the tool calls it makes.
    Assistant {
        calls: Vec<&'a str>,
    },
    /// The result of the tool call `call`.
    ToolResult {
        call: &'a str,
    },
}

/// One message as the planner sees it: its role and its estimated tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanMessage<'a> {
    pub role: PlanRole<'a>,
    pub tokens: usize,
}

/// Everything one plan depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutInput<'a> {
    pub messages: &'a [PlanMessage<'a>],
    /// The estimated tokens of the tool definitions sent with the request.
    pub tool_tokens: usize,
    /// The estimated tokens of the archive stub the cut will insert.
    pub stub_tokens: usize,
    pub trigger: CutTrigger,
}

/// How the kept set compares with L.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// The kept set fits within L.
    WithinLow,
    /// The head fits within L, but the newest exchange does not fit in what
    /// is left; it is kept whole regardless.
    NewestExchangeOverLow,
    /// The head alone (tools, pinned messages, stub) is over L; the newest
    /// exchange is kept whole with it.
    HeadOverLow,
}

/// One cut: which messages stay, which are archived, where the stub goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutPlan {
    kept: Vec<usize>,
    archived: Vec<Range<usize>>,
    stub_slot: usize,
    projected_tokens: usize,
    fill: Fill,
}

impl CutPlan {
    /// The plan that keeps the pinned messages before `tail_start` and
    /// everything from it on, and archives the rest.
    fn new(
        messages: &[PlanMessage<'_>],
        pinned: &[bool],
        tail_start: usize,
        fixed: usize,
        fill: Fill,
    ) -> Self {
        let keeps = |i: &usize| *i >= tail_start || pinned[*i];
        let kept: Vec<usize> = (0..messages.len()).filter(keeps).collect();
        let mut archived: Vec<Range<usize>> = Vec::new();
        for i in (0..tail_start).filter(|i| !keeps(i)) {
            match archived.last_mut() {
                Some(range) if range.end == i => range.end = i + 1,
                Some(_) | None => archived.push(i..i + 1),
            }
        }
        let projected_tokens = fixed.saturating_add(sum(kept.iter().map(|&i| messages[i].tokens)));
        let plan = Self {
            stub_slot: kept.iter().take_while(|&&i| i < tail_start).count(),
            kept,
            archived,
            projected_tokens,
            fill,
        };
        plan.assert_invariants(messages, pinned, fixed);
        plan
    }

    /// Every message is kept or archived, once; the pinned head is kept,
    /// then the stub's slot, then a tail opening on a boundary; no stub is
    /// kept; something besides a stub is archived; no call is split from
    /// its result; the projection counts exactly what is kept.
    fn assert_invariants(&self, messages: &[PlanMessage<'_>], pinned: &[bool], fixed: usize) {
        let mut kept = vec![false; messages.len()];
        self.kept.iter().for_each(|&i| kept[i] = true);
        let archived = self.archived.iter().flat_map(Clone::clone);
        assert!(
            archived.clone().all(|i| !kept[i]),
            "kept or archived, not both"
        );
        assert_eq!(
            self.kept.len() + archived.clone().count(),
            messages.len(),
            "every message placed"
        );
        assert!(self.kept.windows(2).all(|w| w[0] < w[1]), "kept in order");
        assert!(
            pinned.iter().zip(&kept).all(|(p, k)| *k || !*p),
            "the pinned head is kept"
        );
        let tail = &self.kept[self.stub_slot..];
        let tail_start = *tail.first().expect("the newest exchange is kept");
        assert!(
            tail.iter().copied().eq(tail_start..messages.len()),
            "a whole tail"
        );
        assert!(
            self.kept[..self.stub_slot].iter().all(|&i| pinned[i]),
            "the stub follows the head"
        );
        assert!(
            opens_exchange(&messages[tail_start].role),
            "the tail opens an exchange"
        );
        let real = |i: usize| messages[i].role != PlanRole::ArchiveStub;
        assert!(
            self.kept.iter().all(|&i| real(i)),
            "the old stub is not kept"
        );
        assert!(archived.clone().any(real), "the cut archives something");
        for (result, call) in call_of_each_result(messages).into_iter().enumerate() {
            if let Some(call) = call {
                assert_eq!(
                    kept[call], kept[result],
                    "call {call} split from result {result}"
                );
            }
        }
        let projected = fixed.saturating_add(sum(self.kept.iter().map(|&i| messages[i].tokens)));
        assert_eq!(
            self.projected_tokens, projected,
            "the projection counts the kept set"
        );
    }

    /// The indices kept in place, ascending.
    pub fn kept(&self) -> &[usize] {
        &self.kept
    }

    /// The archived ranges of indices, ascending and disjoint.
    pub fn archived(&self) -> &[Range<usize>] {
        &self.archived
    }

    /// The stub's position in the conversation after the cut: it goes
    /// right before `kept()[stub_slot]`.
    pub fn stub_slot(&self) -> usize {
        self.stub_slot
    }

    /// The request's estimated tokens after the cut, the stub included.
    pub fn projected_tokens(&self) -> usize {
        self.projected_tokens
    }

    /// How the kept set compares with L.
    pub fn fill(&self) -> Fill {
        self.fill
    }
}

/// The cut to make before the next request, or `None` when none is due:
/// the trigger is not due, a cut would archive nothing but a previous
/// stub, or it would not save enough.
///
/// The cut keeps the pinned head in place (every system message, the first
/// user message and the latest prompt), puts one stub right after it, and
/// keeps the newest whole exchanges that fit within L, or the newest one
/// alone, whole, when none fits. Everything else is archived, a previous
/// stub with it.
pub fn plan_cut(input: &CutInput<'_>) -> Option<CutPlan> {
    let messages = input.messages;
    let total = input
        .tool_tokens
        .saturating_add(sum(messages.iter().map(|m| m.tokens)));
    if input.trigger.is_due(total) {
        let low = input.trigger.effective_marks().low;
        let pinned = pinned(messages)?;
        let fixed = input.tool_tokens.saturating_add(input.stub_tokens);
        let (tail_start, fill) = tail_start(messages, &pinned, fixed, low)?;
        Some(CutPlan::new(messages, &pinned, tail_start, fixed, fill))
    } else {
        None
    }
}

fn sum(tokens: impl Iterator<Item = usize>) -> usize {
    tokens.fold(0, usize::saturating_add)
}

/// Whether a cut may fall right before a message of this role, when no
/// call/result pair spans it.
fn opens_exchange(role: &PlanRole<'_>) -> bool {
    match role {
        PlanRole::Prompt | PlanRole::User | PlanRole::Assistant { .. } => true,
        PlanRole::System | PlanRole::ArchiveStub | PlanRole::ToolResult { .. } => false,
    }
}

/// Which messages the cut keeps in place whatever it costs: every system
/// message, the first user message and the latest prompt, so a running
/// turn never loses its prompt. `None` without a user message.
fn pinned(messages: &[PlanMessage<'_>]) -> Option<Vec<bool>> {
    let is_user = |m: &PlanMessage<'_>| matches!(m.role, PlanRole::Prompt | PlanRole::User);
    let first = messages.iter().position(is_user)?;
    let latest = messages.iter().rposition(is_user)?;
    let pinned = messages.iter().enumerate().map(|(i, m)| match m.role {
        PlanRole::System => true,
        PlanRole::Prompt | PlanRole::User => i == first || i == latest,
        PlanRole::ArchiveStub | PlanRole::Assistant { .. } | PlanRole::ToolResult { .. } => false,
    });
    Some(pinned.collect())
}

/// The call each tool result answers: the index of the latest assistant
/// message before it that made the call, when there is one.
fn call_of_each_result(messages: &[PlanMessage<'_>]) -> Vec<Option<usize>> {
    let mut issued = std::collections::BTreeMap::new();
    let mut calls = Vec::with_capacity(messages.len());
    for (i, message) in messages.iter().enumerate() {
        calls.push(match &message.role {
            PlanRole::Assistant { calls } => {
                issued.extend(calls.iter().map(|call| (*call, i)));
                None
            }
            PlanRole::ToolResult { call } => issued.get(call).copied(),
            PlanRole::System | PlanRole::Prompt | PlanRole::User | PlanRole::ArchiveStub => None,
        });
    }
    calls
}

/// Whether a cut may fall right before each message: one that opens an
/// exchange, and that no call/result pair spans.
fn boundaries(messages: &[PlanMessage<'_>]) -> Vec<bool> {
    let mut opens = vec![0usize; messages.len() + 1];
    let mut closes = vec![0usize; messages.len() + 1];
    for (result, call) in call_of_each_result(messages).into_iter().enumerate() {
        if let Some(call) = call {
            opens[call + 1] += 1;
            closes[result + 1] += 1;
        }
    }
    let mut open = 0usize;
    let spanned = (0..messages.len()).map(|i| {
        open = open + opens[i] - closes[i];
        open > 0
    });
    let spanned: Vec<bool> = spanned.collect();
    messages
        .iter()
        .zip(spanned)
        .map(|(m, spanned)| opens_exchange(&m.role) && !spanned)
        .collect()
}

/// Where the kept tail starts, and how the kept set compares with `low`:
/// the earliest boundary whose kept set fits, or the newest boundary when
/// none does. The tail holds no stub, and the cut archives at least one
/// message that is not a stub. `fixed` is the tools plus the new stub.
fn tail_start(
    messages: &[PlanMessage<'_>],
    pinned: &[bool],
    fixed: usize,
    low: usize,
) -> Option<(usize, Fill)> {
    let archivable = |i: &usize| match messages[*i].role {
        PlanRole::Prompt
        | PlanRole::User
        | PlanRole::Assistant { .. }
        | PlanRole::ToolResult { .. } => !pinned[*i],
        PlanRole::System | PlanRole::ArchiveStub => false,
    };
    let first_archivable = (0..messages.len()).find(archivable)?;
    let past_stubs = messages
        .iter()
        .rposition(|m| m.role == PlanRole::ArchiveStub)
        .map_or(0, |stub| stub + 1);
    let boundary = boundaries(messages);
    let pinned_tokens = messages
        .iter()
        .zip(pinned)
        .scan(0usize, |before, (m, pinned)| {
            *before = before.saturating_add(if *pinned { m.tokens } else { 0 });
            Some(*before)
        });
    let pinned_before: Vec<usize> = std::iter::once(0).chain(pinned_tokens).collect();
    // The kept set only shrinks as the tail starts later: walk back from
    // the newest message and stop at the first boundary that does not fit.
    let (mut suffix, mut newest, mut fitting) = (0usize, None, None);
    for i in (past_stubs.max(first_archivable + 1)..messages.len()).rev() {
        suffix = suffix.saturating_add(messages[i].tokens);
        if boundary[i] {
            newest.get_or_insert(i);
            let projected = fixed
                .saturating_add(pinned_before[i])
                .saturating_add(suffix);
            if projected <= low {
                fitting = Some(i);
            } else {
                break;
            }
        }
    }
    fitting
        .map(|i| (i, Fill::WithinLow))
        .or(newest.map(|i| (i, Fill::NewestExchangeOverLow)))
}

#[cfg(test)]
#[path = "watermark_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "watermark_split_tests.rs"]
mod split_tests;
