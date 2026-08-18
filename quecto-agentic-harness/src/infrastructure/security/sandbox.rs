// Shared path hook plus dangerous command blocklist and command allowlist.

use std::path::{Path, PathBuf};

/// Shell metacharacters that indicate command chaining/substitution.
/// Includes `\n` because bash treats newlines as command separators equivalent to `;`.
const SHELL_METACHARACTERS: &[&str] = &[";", "\n", "&&", "||", "|", "$(", "`", "<(", ">("];

/// Dangerous command patterns that are always blocked regardless of filesystem path policy.
/// All patterns MUST be lowercase (compared against lowercased input).
const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -r -f /",
    "mkfs ",
    "mkfs.",
    "dd if=/dev/zero",
    "dd if=/dev/random",
    "dd if=/dev/urandom",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "init 0",
    "init 6",
    ":(){ :",
    "chmod -R 777 /",
    // #304: Narrowed from "chown -R " (blocked legitimate workspace-scoped usage and
    // missed the no-space variant -Rroot). Patterns target system-root ownership changes.
    //
    // Known gap: non-recursive `chown root /` (no -R) is not covered here — this is
    // intentional; the denylist only targets recursive forms. Running Quecto in a
    // container is the primary defence for untrusted deployments.
    "chown -r root", // -R root (spaced); also catches -R newroot via substring
    "chown -rroot",  // -Rroot (no space, compact form)
    "chown --recursive root", // GNU long-flag form
    "chown -r 0 /",  // -R UID 0, system root path
    "chown --recursive 0", // GNU long-flag, UID 0
    "chown -r 0:0",  // -R UID 0:GID 0
    "> /dev/sda",
    "wget|sh",
    "wget | sh",
    "curl|sh",
    "curl | sh",
];

/// Shared path-policy hook plus command allowlist/denylist checks.
#[derive(Debug)]
pub struct Sandbox {
    /// The workspace directory used as the default working directory for tools.
    pub workspace: Option<PathBuf>,
    /// Optional command allowlist. When set, only commands whose first token
    /// is in this list are permitted. When `None`, falls back to the denylist.
    command_allowlist: Option<Vec<String>>,
}

impl Clone for Sandbox {
    fn clone(&self) -> Self {
        Self {
            workspace: self.workspace.clone(),
            command_allowlist: self.command_allowlist.clone(),
        }
    }
}

impl Sandbox {
    /// Builder method to set the command allowlist after construction.
    pub fn with_command_allowlist(mut self, allowlist: Option<Vec<String>>) -> Self {
        self.command_allowlist = allowlist;
        self
    }

    /// Create command/path policy with the given workspace.
    pub fn new(workspace: Option<PathBuf>) -> Self {
        Self::with_allowlist(workspace, None)
    }

    /// Create a new sandbox with an explicit command allowlist.
    pub fn with_allowlist(
        workspace: Option<PathBuf>,
        command_allowlist: Option<Vec<String>>,
    ) -> Self {
        Self {
            workspace,
            command_allowlist,
        }
    }

    /// Build command/path policy for an agent/repl entry point from the parsed config.
    ///
    /// The legacy filesystem sandbox mode has been removed; paths are no longer
    /// rejected for being outside the workspace. Command denylist/allowlist
    /// policy remains active.
    pub fn for_agent_workspace(
        config: &crate::infrastructure::config::Config,
        workspace: PathBuf,
    ) -> Self {
        Self::with_allowlist(
            Some(workspace),
            config.agents.defaults.command_allowlist.clone(),
        )
    }

    /// Validate or normalize a file path.
    ///
    /// Filesystem workspace confinement has been removed. This shared hook now
    /// accepts paths without rejecting absolute, home-relative, or parent paths.
    pub fn validate_path(&self, path: &str) -> Result<PathBuf, SandboxError> {
        Ok(Path::new(path).to_path_buf())
    }

    /// Validate that a command is permitted.
    ///
    /// The denylist is ALWAYS checked first, regardless of allowlist configuration.
    /// If a command allowlist is configured, the command's first token must also be
    /// in the list, and shell metacharacters are validated.
    ///
    /// If no allowlist is configured, only the denylist is applied.
    pub fn validate_command(&self, command: &str) -> Result<(), SandboxError> {
        // Always check denylist first — dangerous patterns are never permitted
        self.check_denylist(command)?;

        // If allowlist is configured, also validate against it
        if let Some(ref allowlist) = self.command_allowlist {
            return self.validate_command_allowlist(command, allowlist);
        }

        Ok(())
    }

    /// Check command against the dangerous patterns denylist.
    fn check_denylist(&self, command: &str) -> Result<(), SandboxError> {
        let normalized = normalize_command_for_denylist(command);
        for pattern in DANGEROUS_PATTERNS {
            // Patterns are already lowercase, no need to convert them
            if normalized.contains(pattern) {
                return Err(SandboxError::DangerousPattern(
                    command.to_string(),
                    pattern.to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Validate a command against the allowlist.
    /// Extracts ALL command tokens from the input (splitting on shell metacharacters)
    /// and ensures every one is in the allowlist.
    fn validate_command_allowlist(
        &self,
        command: &str,
        allowlist: &[String],
    ) -> Result<(), SandboxError> {
        // Check for shell metacharacters that could chain commands
        let has_metachar = SHELL_METACHARACTERS.iter().any(|mc| command.contains(mc));

        if has_metachar {
            // Extract all command tokens by splitting on metacharacters
            let tokens = extract_all_command_tokens(command);
            for token in &tokens {
                if !allowlist.iter().any(|a| a == token) {
                    return Err(SandboxError::NotInAllowlist(
                        command.to_string(),
                        token.to_string(),
                    ));
                }
            }
            return Ok(());
        }

        // No metacharacters — just check the first token
        let first_token = command.split_whitespace().next().unwrap_or("");

        if allowlist.iter().any(|a| a == first_token) {
            Ok(())
        } else {
            Err(SandboxError::NotInAllowlist(
                command.to_string(),
                first_token.to_string(),
            ))
        }
    }
}

/// Expand bash `$'...'` escape sequences (hex, octal, unicode) to their literal
/// characters. Also concatenates adjacent `$'...'` and literal fragments like
/// `$'\x72'm` → `rm`. This is a best-effort defence-in-depth pre-processing
/// step; for security-critical deployments, use the command allowlist.
/// Best-effort expansion of bash `$'...'` ANSI-C quoting sequences.
/// Not a full bash parser — intended as a defence-in-depth pre-processing step
/// for the command denylist. Use the command allowlist for security-critical deployments.
pub(crate) fn expand_bash_escapes(command: &str) -> String {
    // Fast path: no $' sequences → return as-is (zero allocation)
    if !command.contains("$'") {
        return command.to_string();
    }

    let mut result = String::with_capacity(command.len());
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Match $'...' ANSI-C quoting
        if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '\'' {
            i += 2; // skip $'
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    let (ch, advance) = expand_escape_sequence(&chars, i);
                    if let Some(ch) = ch {
                        result.push(ch);
                    }
                    i += advance;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if i < chars.len() {
                i += 1; // skip closing '
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Expand a single bash `$'...'` escape sequence starting at `chars[i]` (the `\`).
/// Returns `(Option<char>, bytes_consumed)`.
fn expand_escape_sequence(chars: &[char], i: usize) -> (Option<char>, usize) {
    match chars[i + 1] {
        'x' | 'X' => {
            let hex: String = chars.get(i + 2..).unwrap_or(&[]).iter().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                (Some(byte as char), 2 + hex.len())
            } else {
                (None, 2)
            }
        }
        'u' | 'U' => {
            let max_digits = if chars[i + 1] == 'U' { 8 } else { 4 };
            let hex: String = chars
                .get(i + 2..)
                .unwrap_or(&[])
                .iter()
                .take(max_digits)
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            let ch = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
            // Invalid codepoints (surrogates) are silently dropped —
            // intentional for denylist safety.
            (ch, 2 + hex.len())
        }
        '0'..='7' => {
            let oct: String = chars
                .get(i + 1..)
                .unwrap_or(&[])
                .iter()
                .take(3)
                .take_while(|c| matches!(c, '0'..='7'))
                .collect();
            let ch = u8::from_str_radix(&oct, 8).ok().map(|b| b as char);
            (ch, 1 + oct.len())
        }
        'n' => (Some('\n'), 2),
        't' => (Some('\t'), 2),
        '\\' => (Some('\\'), 2),
        other => (Some(other), 2),
    }
}

/// Extract string literal values from variable assignments (e.g. `cmd='rm -rf /'`)
/// and append them to the command for denylist scanning.
/// Splits on all shell metacharacters (`;`, `&&`, `||`, `|`, newlines).
fn extract_string_literals(command: &str) -> String {
    // Fast path: no '=' → no assignments
    if !command.contains('=') {
        return String::new();
    }

    let mut extra = String::new();
    // Normalize metacharacters (`&&`, `||`, `|`, newline) to a common `;`
    // separator in a single scan, instead of four sequential `.replace()`
    // passes that each allocated a fresh command-sized string (#996 item 7).
    let bytes = command.as_bytes();
    let mut normalized = String::with_capacity(command.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'&' if i + 1 < bytes.len() && bytes[i + 1] == b'&' => {
                normalized.push(';');
                i += 2;
            }
            b'|' => {
                normalized.push(';');
                // Collapse `||` to a single separator, matching the old
                // replace("||", ";") then replace("|", ";") behaviour.
                i += if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    2
                } else {
                    1
                };
            }
            b'\n' => {
                normalized.push(';');
                i += 1;
            }
            _ => {
                let ch = command[i..].chars().next().unwrap_or('\0');
                normalized.push(ch);
                i += ch.len_utf8();
            }
        }
    }

    for segment in normalized.split(';') {
        let trimmed = segment.trim();
        if let Some(eq_pos) = trimmed.find('=') {
            let before = &trimmed[..eq_pos];
            if before
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !before.is_empty()
            {
                let after = trimmed[eq_pos + 1..].trim();
                // Strip single/double quotes
                let unquoted = if after.len() >= 2
                    && ((after.starts_with('\'') && after.ends_with('\''))
                        || (after.starts_with('"') && after.ends_with('"')))
                {
                    &after[1..after.len() - 1]
                } else {
                    after
                };
                if !unquoted.is_empty() {
                    extra.push(' ');
                    extra.push_str(unquoted);
                }
            }
        }
    }
    extra
}

fn normalize_command_for_denylist(command: &str) -> String {
    // Pre-process: expand bash escape sequences and extract string literals
    let expanded = expand_bash_escapes(command);
    let literals = extract_string_literals(&expanded);
    let combined = if literals.is_empty() {
        expanded
    } else {
        format!("{expanded}{literals}")
    };

    let mut normalized = String::with_capacity(combined.len());
    let mut in_whitespace = false;

    for ch in combined.chars() {
        for lower in ch.to_lowercase() {
            if lower.is_whitespace() {
                if !in_whitespace && !normalized.is_empty() {
                    normalized.push(' ');
                }
                in_whitespace = true;
            } else {
                normalized.push(lower);
                in_whitespace = false;
            }
        }
    }

    if normalized.ends_with(' ') {
        normalized.pop();
    }

    normalized
}

/// Returns true if `bytes[i..]` starts with a two-byte shell metacharacter
/// (`&&`, `||`, `<(`, `>(`, `$(`).
#[inline]
fn is_two_byte_meta(bytes: &[u8], i: usize) -> bool {
    if i + 1 >= bytes.len() {
        return false;
    }
    let (b, next) = (bytes[i], bytes[i + 1]);
    matches!(
        (b, next),
        (b'&', b'&') | (b'|', b'|') | (b'<', b'(') | (b'>', b'(') | (b'$', b'(')
    )
}

/// Returns true if `b` is a single-byte shell metacharacter (`;`, `|`, `` ` ``, `\n`).
///
/// Note: callers must check `is_two_byte_meta` **before** this function when
/// consuming (not just detecting) a metacharacter, because `|` is a prefix of
/// the two-byte sequence `||`. The token-collection loop detects via this
/// function; the consume loop checks two-byte first. This ordering is
/// intentional and must be preserved.
///
/// `\n` is included because bash treats newlines as command separators
/// equivalent to `;`.
#[inline]
fn is_one_byte_meta(b: u8) -> bool {
    matches!(b, b';' | b'|' | b'`' | b'\n')
}

/// Advance `i` past a metacharacter boundary; returns the updated index.
#[inline]
fn skip_meta(bytes: &[u8], i: usize) -> usize {
    if is_two_byte_meta(bytes, i) {
        i + 2
    } else {
        i + 1
    }
}

/// Extract all command tokens (first words) from a shell command string,
/// splitting on metacharacters like `;`, `|`, `&&`, `||`.
///
/// Uses a single-pass byte scanner that avoids intermediate `String`
/// allocations (#307).
pub(crate) fn extract_all_command_tokens(command: &str) -> Vec<String> {
    let bytes = command.as_bytes();
    let mut tokens = Vec::with_capacity(4); // most commands have 1–5 segments
    let mut i = 0;

    while i < bytes.len() {
        // Skip whitespace between segments.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        // Collect the first token of this segment (stop at whitespace or meta).
        let token_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !is_one_byte_meta(bytes[i])
            && !is_two_byte_meta(bytes, i)
        {
            i += 1;
        }
        if i > token_start {
            tokens.push(command[token_start..i].to_string());
        }

        // Consume the rest of the segment up to (and including) the next meta.
        while i < bytes.len() {
            if is_two_byte_meta(bytes, i) || is_one_byte_meta(bytes[i]) {
                i = skip_meta(bytes, i);
                break;
            }
            i += 1;
        }
    }

    tokens
}

#[derive(Debug)]
pub enum SandboxError {
    /// Command matches a dangerous pattern.
    DangerousPattern(String, String),
    /// Command is not in the allowlist.
    NotInAllowlist(String, String),
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxError::DangerousPattern(cmd, pattern) => {
                write!(
                    f,
                    "command '{}' matches dangerous pattern '{}'",
                    cmd, pattern
                )
            }
            SandboxError::NotInAllowlist(cmd, token) => {
                write!(f, "command '{}': '{}' is not in allowlist", cmd, token)
            }
        }
    }
}

impl std::error::Error for SandboxError {}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
