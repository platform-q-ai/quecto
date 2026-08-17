use crate::domain::error::DomainError;
use crate::domain::message::ThinkingBlock;
use serde::ser::{SerializeSeq, SerializeStruct};

pub const MAX_VISIBLE_THINKING_BYTES: usize = 8 * 1024 * 1024;

pub fn append_visible_thinking(
    target: &mut String,
    fragment: &str,
    label: &str,
) -> Result<(), DomainError> {
    append_visible_thinking_with_limit(target, fragment, MAX_VISIBLE_THINKING_BYTES, label)
}

pub fn append_visible_thinking_with_limit(
    target: &mut String,
    fragment: &str,
    limit: usize,
    label: &str,
) -> Result<(), DomainError> {
    let new_len = target.len().checked_add(fragment.len()).ok_or_else(|| {
        DomainError::Provider(format!(
            "{label} visible thinking exceeds {limit} byte limit"
        ))
    })?;
    if new_len > limit {
        return Err(DomainError::Provider(format!(
            "{label} visible thinking exceeds {limit} byte limit"
        )));
    }
    target.push_str(fragment);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VisibleThinkingPageBlock {
    Text { text: String },
    Redacted,
}

impl serde::Serialize for VisibleThinkingPageBlock {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text { text } => {
                let mut s = serializer.serialize_struct("Thinking", 2)?;
                s.serialize_field("kind", "text")?;
                s.serialize_field("text", text)?;
                s.end()
            }
            Self::Redacted => {
                let mut s = serializer.serialize_struct("Thinking", 1)?;
                s.serialize_field("kind", "redacted")?;
                s.end()
            }
        }
    }
}

pub fn visible_thinking_len(blocks: &[ThinkingBlock]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            ThinkingBlock::Normal { thinking, .. } => thinking.len(),
            ThinkingBlock::Redacted { .. } => 1,
        })
        .sum()
}

pub fn visible_thinking_page(
    blocks: &[ThinkingBlock],
    offset: usize,
    end: usize,
) -> Vec<VisibleThinkingPageBlock> {
    let mut cursor = 0usize;
    let mut out = Vec::new();
    for block in blocks {
        match block {
            ThinkingBlock::Normal { thinking, .. } => {
                let block_start = cursor;
                let block_end = cursor + thinking.len();
                if end > block_start && offset < block_end {
                    let s = nearest_char_boundary_at_or_before(
                        thinking,
                        offset.saturating_sub(block_start),
                    );
                    let e = nearest_char_boundary_at_or_before(
                        thinking,
                        end.min(block_end) - block_start,
                    );
                    if e > s {
                        out.push(VisibleThinkingPageBlock::Text {
                            text: thinking[s..e].to_string(),
                        });
                    }
                }
                cursor = block_end;
            }
            ThinkingBlock::Redacted { .. } => {
                if offset <= cursor && cursor < end {
                    out.push(VisibleThinkingPageBlock::Redacted);
                }
                cursor += 1;
            }
        }
    }
    out
}

fn nearest_char_boundary_at_or_before(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

pub struct VisibleThinkingBlocksView<'a>(pub &'a [ThinkingBlock]);

struct VisibleThinkingBlockView<'a>(&'a ThinkingBlock);

impl serde::Serialize for VisibleThinkingBlockView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ThinkingBlock::Normal { thinking, .. } => VisibleThinkingPageBlock::Text {
                text: thinking.clone(),
            }
            .serialize(serializer),
            ThinkingBlock::Redacted { .. } => {
                VisibleThinkingPageBlock::Redacted.serialize(serializer)
            }
        }
    }
}

impl serde::Serialize for VisibleThinkingBlocksView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for block in self.0 {
            seq.serialize_element(&VisibleThinkingBlockView(block))?;
        }
        seq.end()
    }
}
