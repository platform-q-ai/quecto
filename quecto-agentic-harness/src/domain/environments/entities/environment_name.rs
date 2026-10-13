//! The allowlist an environment name must satisfy to be created. A name
//! is shown in `quecto container ls`'s table, in logs and in `kill
//! <ref|name>`; a TAB, LF or space in it would break every one of them.

/// The longest name, in bytes.
pub const MAX_ENVIRONMENT_NAME_BYTES: usize = 64;

/// Whether `name` may name a new environment: 1 to
/// [`MAX_ENVIRONMENT_NAME_BYTES`] bytes of ASCII letters, digits, `-`, `_`
/// and `.`, starting with a letter or digit. Admitted by shape; anything
/// else (whitespace, control characters, `/`, non-ASCII) is refused.
pub fn is_safe_environment_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let length_admitted = (1..=MAX_ENVIRONMENT_NAME_BYTES).contains(&bytes.len());
    let first_admitted = bytes.first().is_some_and(u8::is_ascii_alphanumeric);
    let rest_admitted = bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    length_admitted && first_admitted && rest_admitted
}

#[cfg(test)]
#[path = "environment_name_tests.rs"]
mod tests;
