//! The allowlist an environment name must satisfy to be created. A name
//! is shown in `quecto container ls`'s table, in logs and in `kill
//! <ref|name>`; a TAB, LF or space in it would break every one of them,
//! and a name shaped like a ref (`C7`) would be read as that ref.
//!
//! A record written before the rule keeps whatever name it holds
//! (`EnvironmentRecord::name` stays a `String`); only a new environment's
//! name is an [`EnvironmentName`].

use std::fmt;

/// The longest name, in bytes.
pub const MAX_ENVIRONMENT_NAME_BYTES: usize = 64;

/// Whether `name` may name a new environment: 1 to
/// [`MAX_ENVIRONMENT_NAME_BYTES`] bytes of ASCII letters, digits, `-`, `_`
/// and `.`, starting with a letter or digit, and not shaped like a ref
/// (`C` followed by digits only). Admitted by shape; anything else
/// (whitespace, control characters, `/`, non-ASCII) is refused.
pub fn is_safe_environment_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let length_admitted = (1..=MAX_ENVIRONMENT_NAME_BYTES).contains(&bytes.len());
    let first_admitted = bytes.first().is_some_and(u8::is_ascii_alphanumeric);
    let rest_admitted = bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    length_admitted && first_admitted && rest_admitted && !is_ref_shaped(name)
}

/// `C` followed by one or more ASCII digits: what `kill <ref|name>` reads
/// as a ref (judged by shape, so a number too large to parse counts too).
fn is_ref_shaped(name: &str) -> bool {
    name.strip_prefix('C')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// A new environment's name: only a name [`is_safe_environment_name`]
/// admits can be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EnvironmentName {
    type Error = UnsafeEnvironmentName;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if is_safe_environment_name(&name) {
            return Ok(Self(name));
        }
        Err(UnsafeEnvironmentName)
    }
}

impl From<EnvironmentName> for String {
    fn from(name: EnvironmentName) -> Self {
        name.0
    }
}

/// A name outside the allowlist. Its text states the rule (the caller
/// names the field it came from).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsafeEnvironmentName;

impl fmt::Display for UnsafeEnvironmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "must be 1-64 characters, each an ASCII letter, digit, '-', '_' or '.', starting with an ASCII letter or digit, and not shaped like a ref (C followed by digits)",
        )
    }
}

impl std::error::Error for UnsafeEnvironmentName {}

#[cfg(test)]
#[path = "environment_name_tests.rs"]
mod tests;
