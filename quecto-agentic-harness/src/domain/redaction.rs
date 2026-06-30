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

/// [`redact_secrets`] then bound the result to `max_len` bytes, appending an
/// ellipsis marker when truncated (truncation respects char boundaries).
pub(crate) fn redact_and_bound(input: &str, max_len: usize) -> String {
    let redacted = redact_secrets(input);
    if redacted.len() > max_len {
        let mut end = max_len;
        while !redacted.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…[truncated]", &redacted[..end])
    } else {
        redacted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_named_and_prefixed_secrets() {
        assert_eq!(
            redact_secrets("Authorization: Bearer abc.def"),
            "Authorization: [REDACTED]"
        );
        assert_eq!(redact_secrets("sk-livedeadbeef0001"), "[REDACTED]");
        assert_eq!(redact_secrets("token=hunter2"), "[REDACTED]");
        assert_eq!(redact_secrets("--api-key=topsecret"), "--[REDACTED]");
    }

    #[test]
    fn scrubs_positional_provider_tokens() {
        assert_eq!(redact_secrets("AKIAIOSFODNN7EXAMPLE"), "[REDACTED]");
        assert_eq!(
            redact_secrets("ghp_0123456789abcdef0123456789abcdef0123"),
            "[REDACTED]"
        );
        assert_eq!(redact_secrets("xoxb-0123456789-abcdefghij"), "[REDACTED]");
    }

    #[test]
    fn preserves_non_secret_text() {
        assert_eq!(redact_secrets("ls -la /tmp"), "ls -la /tmp");
        assert_eq!(redact_secrets("usage_limit_reached"), "usage_limit_reached");
    }

    #[test]
    fn redact_and_bound_truncates() {
        let long = format!("token=secret {}", "x".repeat(3000));
        let out = redact_and_bound(&long, 2000);
        assert!(out.starts_with("[REDACTED]"));
        assert!(out.ends_with("…[truncated]"));
        assert!(out.len() <= 2000 + "…[truncated]".len());
    }
}
