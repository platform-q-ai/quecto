// #2348: the size-aware collapse. A tool result over `over_tokens` that
// the model has seen for `after_turns` requests collapses to its recall
// stub, whatever the count dial says. A count dial is blind to size: in
// the owner's run one 17k-token bash output rode 98 coordinator requests
// (31% of its input) while a dial of 50 or 100 waited for small results
// to pile up. Pure policy over the message list: no retention handle, no
// store.

use crate::domain::message::Message;

/// The size-aware collapse dial (#2348): a tool result estimated over
/// `over_tokens` collapses to its recall stub once `after_turns` model
/// responses have followed it (each one answered a request that carried
/// it in full). `after_turns` is at least 1: an unseen result is never
/// stubbed (#2213).
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

/// Collapse every spilled, seen tool result over the dial's size that the
/// model has seen for its turns. Returns how many it collapsed.
pub fn collapse_large_results(_messages: &mut [Message], _dial: LargeResultCollapse) -> usize {
    0
}

#[cfg(test)]
#[path = "context_pruning_large_results_tests.rs"]
mod tests;
