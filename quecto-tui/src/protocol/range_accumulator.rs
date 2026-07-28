#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RangeUpdate {
    Continue {
        content: String,
        next_offset: usize,
        content_len: Option<usize>,
    },
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
    expected_content_len: Option<usize>,
}

impl RangeAccumulator {
    pub(crate) fn new_with_expected_len(
        content: String,
        offset: usize,
        expected_content_len: Option<usize>,
    ) -> Self {
        Self {
            content,
            offset,
            expected_content_len,
        }
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
        let advertised_content_len = data
            .get("contentLength")
            .and_then(|v| v.as_u64())
            .and_then(|n| usize::try_from(n).ok());
        if advertised_content_len
            .zip(self.expected_content_len)
            .is_some_and(|(advertised, expected)| advertised != expected)
        {
            return Err(RangeError::LengthMismatch);
        }
        let content_len = self.expected_content_len.or(advertised_content_len);
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
            if next_offset <= response_offset || next_offset != self.content.len() {
                return Err(RangeError::InvalidProgress);
            }
            if content_len.is_some_and(|len| next_offset > len || self.content.len() > len) {
                return Err(RangeError::InvalidProgress);
            }
            Ok(RangeUpdate::Continue {
                content: self.content,
                next_offset,
                content_len,
            })
        } else if content_len.is_none_or(|len| self.content.len() == len) {
            Ok(RangeUpdate::Complete(self.content))
        } else {
            Err(RangeError::LengthMismatch)
        }
    }
}

#[cfg(test)]
#[path = "range_accumulator_tests.rs"]
mod range_accumulator_tests;
