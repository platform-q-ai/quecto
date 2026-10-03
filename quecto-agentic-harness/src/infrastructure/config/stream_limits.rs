//! A provider's configured stream limits (#2433): `stream_idle_seconds` and
//! `stream_progress_seconds` on a config.json provider entry or
//! `openai_compatible` endpoint, flattened into it, and their validation.
//! Split from `config.rs` to keep that file within the line-count gate.
use serde::{Deserialize, Serialize};

use crate::infrastructure::providers::stream_idle::{StreamIdle, StreamLimits};

/// The stream limits of one provider entry, in seconds; unset is the
/// default. Serialized flat into the entry.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamLimitsConfig {
    /// The stream idle limit, within 30–1800; unset is the default 300.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_idle_seconds: Option<u64>,
    /// The stream progress limit, within 60–3600; unset is the default 300.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_progress_seconds: Option<u64>,
}

impl StreamLimitsConfig {
    /// The limits as the providers read them.
    pub fn limits(self) -> StreamLimits {
        StreamLimits {
            idle_seconds: self.stream_idle_seconds,
            progress_seconds: self.stream_progress_seconds,
        }
    }

    /// The bounds of the provider configured at `setting` (such as
    /// `providers.openai`), or the error naming the limit out of range.
    pub fn bounds(self, setting: &str) -> Result<StreamIdle, String> {
        StreamIdle::configured_for(setting, self.limits())
    }
}
