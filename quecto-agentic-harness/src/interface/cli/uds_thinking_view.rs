use crate::domain::message::ThinkingBlock;
use serde::ser::{SerializeSeq, SerializeStruct};

pub(super) struct VisibleThinkingBlocksView<'a>(pub(super) &'a [ThinkingBlock]);

struct VisibleThinkingBlockView<'a>(&'a ThinkingBlock);

impl serde::Serialize for VisibleThinkingBlockView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ThinkingBlock::Normal { thinking, .. } => {
                let mut s = serializer.serialize_struct("Thinking", 2)?;
                s.serialize_field("kind", "text")?;
                s.serialize_field("text", thinking)?;
                s.end()
            }
            ThinkingBlock::Redacted { .. } => {
                let mut s = serializer.serialize_struct("Thinking", 1)?;
                s.serialize_field("kind", "redacted")?;
                s.end()
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
