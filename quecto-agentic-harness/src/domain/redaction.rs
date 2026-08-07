//! Shared secret redaction for audit logs and subagent error surfacing.
//!
//! A single source of truth for scrubbing credential-shaped spans out of
//! strings before they are persisted (audit log, #790) or propagated to the
//! parent agent context (subagent error cause, #752). Keeping one regex avoids
//! the two-copy drift risk where tightening one redactor silently leaves the
//! other leaking. Pure; no I/O.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// A string guaranteed to have passed through [`redact_secrets`].
///
/// Its only text constructors ([`Redacted::new`], `From<&str>`/`From<String>`)
/// redact on the way in, so a value of this type cannot carry an un-scrubbed
/// secret-shaped span. This turns the "redact before persistence" invariant for
/// [`crate::domain::audit::AuditEvent::ProviderError`] into a property the type
/// system enforces rather than a convention an emitter could bypass by building
/// the raw variant directly (#937 review). Serializes transparently as the inner
/// string, so the on-disk/wire format is unchanged. Deserialization (reading the
/// audit file back) trusts the already-redacted on-disk value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Redacted(String);

impl Redacted {
    /// Redact `raw` and wrap it. The sole entry point from untrusted text.
    pub(crate) fn new(raw: &str) -> Self {
        Self(redact_secrets(raw))
    }

    /// Borrow the redacted contents as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Redacted {
    fn from(raw: &str) -> Self {
        Self::new(raw)
    }
}

impl From<String> for Redacted {
    fn from(raw: String) -> Self {
        Self::new(&raw)
    }
}

impl std::ops::Deref for Redacted {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Credential-shaped patterns scrubbed to `[REDACTED]`.
///
/// Covers both named spans (`Bearer <tok>`, `<api_key|token|password|secret|
/// access_token>=<value>`) and positionally-passed provider tokens that have
/// recognisable prefixes (`sk-`, AWS `AKIA`, GitHub `gh[pousr]_`, Slack
/// `xox[baprs]-`, Google `AIza`). This is best-effort: tokens with no
/// distinguishing shape (bare passwords passed positionally, opaque JWTs) can
/// still slip through, so callers must not treat output as guaranteed clean.
static PATTERNS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(bearer\s+\S+|sk-[A-Za-z0-9_-]{8,}|AKIA[0-9A-Z]{12,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AIza[A-Za-z0-9_-]{20,}|(?:api[_-]?key|token|password|secret|access[_-]?token)\s*[=:]\s*\S+)",
    )
    .expect("static redaction regex is valid")
});

/// Replace known secret shapes in `input` with `[REDACTED]`.
///
/// Non-secret tokens are preserved so the redacted string stays useful.
pub(crate) fn redact_secrets(input: &str) -> String {
    PATTERNS.replace_all(input, "[REDACTED]").into_owned()
}

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod tests;
#[cfg(test)]
mod redacted_cov_tests {
    use super::*;

    #[test]
    fn redacted_from_string_and_display_scrub_secret() {
        let redacted = Redacted::from(String::from("prefix token=hunter2 suffix"));
        assert_eq!(redacted.as_str(), "prefix [REDACTED] suffix");
        assert_eq!(redacted.to_string(), "prefix [REDACTED] suffix");
        assert_eq!(&*redacted, "prefix [REDACTED] suffix");
    }
}
