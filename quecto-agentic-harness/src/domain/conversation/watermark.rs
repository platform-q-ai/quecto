//! The watermark cut planner (#2402).
//!
//! A prompt cache matches the longest identical prefix of a request, so the
//! watermark context only appends. When the request reaches the high mark
//! (H), one deep cut takes it down to the low mark (L). This module plans
//! that cut; it is pure: no I/O, no tokenizer. Token estimates come in.

use std::ops::Range;

/// The two marks, in tokens of the whole request: messages plus tool
/// definitions. Always `0 < low < high`.
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
    /// the ceiling lowers H, L scales by the same ratio. `None` when the
    /// ceiling leaves no positive low mark.
    pub fn under_ceiling(self, _ceiling: usize) -> Option<Self> {
        Some(self)
    }
}

/// What the planner knows about one message: its role, and its links to
/// tool calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRole<'a> {
    System,
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
    pub marks: Watermark,
    /// The most the request may hold: the model window less the output
    /// reserve, or the configured maximum.
    pub ceiling: usize,
    /// The request's tokens right after the previous cut, if one was made.
    pub after_last_cut: Option<usize>,
}

/// One cut: which messages stay, which are archived, where the stub goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutPlan {
    kept: Vec<usize>,
    archived: Vec<Range<usize>>,
    stub_slot: usize,
    projected_tokens: usize,
    over_low_mark: bool,
}

impl CutPlan {
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

    /// Whether even the newest exchange alone does not fit within L; it is
    /// kept whole regardless.
    pub fn over_low_mark(&self) -> bool {
        self.over_low_mark
    }
}

/// The cut to make before the next request, or `None` when the request is
/// below the effective high mark or no cut is due.
pub fn plan_cut(_input: &CutInput<'_>) -> Option<CutPlan> {
    None
}

#[cfg(test)]
#[path = "watermark_tests.rs"]
mod tests;
