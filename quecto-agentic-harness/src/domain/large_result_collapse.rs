//! The size-aware collapse dial (#2348): a value the configuration sets
//! and the context-pruning policy reads. No behaviour of its own.

/// A tool result estimated over `over_tokens` collapses to its recall stub
/// once `after_turns` model responses have followed it (each one answered
/// a request that carried it in full). `after_turns` is at least 1 in
/// effect: an unseen result is never stubbed (#2213).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LargeResultCollapse {
    pub over_tokens: usize,
    pub after_turns: u32,
}

impl LargeResultCollapse {
    /// The rule switched off: no result is ever over `usize::MAX` tokens.
    pub const DISABLED: Self = Self {
        over_tokens: usize::MAX,
        after_turns: u32::MAX,
    };
}
