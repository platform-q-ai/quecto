use crate::domain::message::ThinkingBlock;

pub(crate) fn blocks_to_json(blocks: &[ThinkingBlock]) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .map(|block| match block {
            ThinkingBlock::Normal { thinking, .. } => {
                serde_json::json!({"text": thinking, "type": "thinking"})
            }
            ThinkingBlock::Redacted { .. } => {
                serde_json::json!({"text": "[Redacted thinking]", "type": "redacted"})
            }
        })
        .collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct VisibleThinkingView<'a> {
    text: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
}

pub(crate) struct BlocksView<'a>(pub(crate) &'a [ThinkingBlock]);

impl serde::Serialize for BlocksView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for block in self.0 {
            match block {
                ThinkingBlock::Normal { thinking, .. } => {
                    seq.serialize_element(&VisibleThinkingView {
                        text: thinking,
                        kind: "thinking",
                    })?;
                }
                ThinkingBlock::Redacted { .. } => {
                    seq.serialize_element(&VisibleThinkingView {
                        text: "[Redacted thinking]",
                        kind: "redacted",
                    })?;
                }
            }
        }
        seq.end()
    }
}
