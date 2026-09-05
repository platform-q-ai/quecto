// Pre-#1620 whole-string substring scan, retained verbatim as the explicit
// fallback for syntax the execution-aware parser cannot resolve.

// ---------------------------------------------------------------------------
// Legacy whole-string substring scan — fallback for unresolved syntax only.
// ---------------------------------------------------------------------------

/// Patterns for the fallback scan. All lowercase (compared against lowercased
/// input). This is the pre-#1620 list, retained verbatim so that behaviour on
/// dynamic syntax is exactly what it was before.
const LEGACY_PATTERNS: &[&str] = &[
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
    "chmod -r 777 /",
    "chown -r root",
    "chown -rroot",
    "chown --recursive root",
    "chown -r 0 /",
    "chown --recursive 0",
    "chown -r 0:0",
    "> /dev/sda",
    "wget|sh",
    "wget | sh",
    "curl|sh",
    "curl | sh",
];

/// Whole-string scan: returns the first matching legacy pattern.
pub(crate) fn legacy_substring_scan(command: &str) -> Option<&'static str> {
    let normalized = normalize_for_legacy_scan(command);
    LEGACY_PATTERNS
        .iter()
        .copied()
        .find(|p| normalized.contains(p))
}

/// Expand bash `$'...'` ANSI-C quoting into literal characters, concatenating
/// adjacent fragments (`$'\x72'm` → `rm`).
pub(crate) fn expand_bash_escapes(command: &str) -> String {
    if !command.contains("$'") {
        return command.to_string();
    }
    let mut result = String::with_capacity(command.len());
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '\'' {
            i += 2;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    let (ch, advance) = super::ansi_c::expand_escape_sequence(&chars, i);
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
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Collect the values of `NAME=value` assignments so that
/// `cmd='rm -rf /'; $cmd` is visible to the substring scan.
fn extract_string_literals(command: &str) -> String {
    if !command.contains('=') {
        return String::new();
    }
    let mut extra = String::new();
    let normalized = command
        .replace("&&", ";")
        .replace("||", ";")
        .replace(['|', '\n'], ";");
    for segment in normalized.split(';') {
        let trimmed = segment.trim();
        if let Some(eq_pos) = trimmed.find('=') {
            let before = &trimmed[..eq_pos];
            if !before.is_empty()
                && before
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                let after = trimmed[eq_pos + 1..].trim();
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

fn normalize_for_legacy_scan(command: &str) -> String {
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
