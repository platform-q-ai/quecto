//! Secret redaction of configuration read-outs (#2024): a `config get`
//! lands in a transcript, a log, or a model's context, so leaves whose
//! key names a credential read as [`REDACTED`] unless the caller asks for
//! them. The rule is by key shape, not by section: `providers.*.api_key`
//! is the known case, but an unknown key that carries a token is one too.

use serde_json::Value;

/// What a redacted leaf reads as.
pub const REDACTED: &str = "<redacted>";

/// Key names that are a secret, whatever the section.
const SECRET_KEYS: &[&str] = &["api_key", "apikey", "token", "secret", "password"];

/// Key suffixes that mark a secret (`openai_key`, `refresh_token`).
const SECRET_SUFFIXES: &[&str] = &["_key", "_token"];

/// Whether a key names a credential. Case-insensitive, so `apiKey` and
/// `API_KEY` count.
pub fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEYS.contains(&key.as_str())
        || SECRET_SUFFIXES.iter().any(|suffix| key.ends_with(suffix))
}

/// Replace every scalar leaf under a secret-shaped key with [`REDACTED`],
/// recursing through objects and arrays, and return how many were
/// replaced. `key` is the name `value` itself sits under, when known (a
/// read narrowed to `providers.openai.api_key` yields the bare leaf); a
/// `null` is left as it is — it discloses nothing.
pub fn redact_secret_leaves(value: &mut Value, key: Option<&str>) -> usize {
    match value {
        Value::Object(map) => map
            .iter_mut()
            .map(|(key, child)| redact_secret_leaves(child, Some(key)))
            .sum(),
        Value::Array(items) => items
            .iter_mut()
            .map(|item| redact_secret_leaves(item, key))
            .sum(),
        Value::Null => 0,
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            if key.is_some_and(is_secret_key) {
                *value = Value::String(REDACTED.to_string());
                1
            } else {
                0
            }
        }
    }
}

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod tests;
