#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RangeUpdate {
    Continue { content: String, next_offset: usize },
    Complete(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RangeError {
    MissingContent,
    OffsetMismatch,
    MissingNextOffset,
    InvalidProgress,
    LengthMismatch,
}

#[derive(Debug, Clone)]
pub(crate) struct RangeAccumulator {
    content: String,
    offset: usize,
}

impl RangeAccumulator {
    pub(crate) fn new(content: String, offset: usize) -> Self {
        Self { content, offset }
    }

    pub(crate) fn apply(mut self, data: &serde_json::Value) -> Result<RangeUpdate, RangeError> {
        let content = data
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or(RangeError::MissingContent)?;
        let response_offset = data
            .get("offset")
            .and_then(|v| v.as_u64())
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        if response_offset != self.offset {
            return Err(RangeError::OffsetMismatch);
        }
        self.content.push_str(content);
        let content_len = data
            .get("contentLength")
            .and_then(|v| v.as_u64())
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(self.content.len());
        let has_more = data
            .get("hasMoreContent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if has_more {
            let next_offset = data
                .get("nextOffset")
                .and_then(|v| v.as_u64())
                .and_then(|n| usize::try_from(n).ok())
                .ok_or(RangeError::MissingNextOffset)?;
            if next_offset <= response_offset
                || next_offset > content_len
                || self.content.len() > content_len
            {
                return Err(RangeError::InvalidProgress);
            }
            Ok(RangeUpdate::Continue {
                content: self.content,
                next_offset,
            })
        } else if self.content.len() == content_len {
            Ok(RangeUpdate::Complete(self.content))
        } else {
            Err(RangeError::LengthMismatch)
        }
    }
}
