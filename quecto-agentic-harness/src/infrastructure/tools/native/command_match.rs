//! Command pattern matching for workflow guards.
//!
//! Matches bash commands against configurable patterns like `"git commit"`,
//! `"gh pr merge"`, `"cargo publish"`. Handles chained commands, flags,
//! subshells, and backtick expressions.

/// Extract the command string from bash tool JSON arguments.
pub(crate) fn extract_bash_command(arguments: &str) -> String {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
        .unwrap_or_default()
}

/// Check if a bash command matches any of the given command patterns.
///
/// Each pattern is `"binary subcommand"` (e.g. `"git commit"`, `"gh pr merge"`).
/// Handles chained commands (`&&`, `||`, `;`, newlines), flags between
/// binary and subcommand, and subshells (`$(...)`, backticks).
/// Convenience wrapper that parses patterns and matches in one call.
/// Used by tests only; production code uses `parse_patterns` + `command_matches_parsed`
/// to avoid per-call allocation.
#[cfg(test)]
pub fn command_matches_patterns(command: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }

    let parsed = parse_patterns(patterns);
    command_matches_parsed(command, &parsed)
}

/// Parse command pattern strings into (binary, subcmd_tokens) pairs.
pub(crate) fn parse_patterns(patterns: &[String]) -> Vec<(String, Vec<String>)> {
    patterns
        .iter()
        .map(|p| {
            let parts: Vec<&str> = p.split_whitespace().collect();
            let binary = parts.first().unwrap_or(&"").to_lowercase();
            let subcmds: Vec<String> = parts.iter().skip(1).map(|s| s.to_lowercase()).collect();
            (binary, subcmds)
        })
        .collect()
}

/// Strip content inside single and double quotes, replacing with spaces.
///
/// Handles nested quotes (single inside double and vice versa) by only
/// tracking the outermost quote character. Escaped quotes (`\"`, `\'`)
/// inside double-quoted regions are skipped.
fn strip_quoted_regions(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_quote: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        match in_quote {
            None => {
                if b == b'\'' || b == b'"' {
                    in_quote = Some(b);
                    out.push(' '); // placeholder so tokens don't merge
                } else {
                    out.push(b as char);
                }
            }
            Some(q) => {
                if b == b'\\' && q == b'"' && i + 1 < bytes.len() {
                    // Skip escaped char inside double quotes
                    i += 2;
                    continue;
                }
                if b == q {
                    in_quote = None;
                }
                // Consume quoted content (don't emit it)
            }
        }
        i += 1;
    }
    out
}

/// Inner matching logic against pre-parsed patterns.
pub(crate) fn command_matches_parsed(command: &str, patterns: &[(String, Vec<String>)]) -> bool {
    // Strip quoted regions first so data inside strings doesn't trigger
    // false positives (#405).
    let unquoted = strip_quoted_regions(command);
    // Lowercase once for case-insensitive matching at all levels
    let lower = unquoted.to_lowercase();

    for segment in lower.split(&['&', '|', ';', '\n'][..]) {
        let segment = segment.trim();
        if segment.is_empty() || segment.starts_with('#') {
            continue;
        }

        if segment_matches_any(segment, patterns) {
            return true;
        }
    }

    contains_in_subshell_lowered(&lower, patterns)
}

/// Check if a single command segment matches any pattern.
fn segment_matches_any(segment: &str, patterns: &[(String, Vec<String>)]) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();

    for (binary, subcmds) in patterns {
        if matches_binary_subcommand(&tokens, binary, subcmds) {
            return true;
        }
    }
    false
}

/// Check if tokens contain `binary [flags...] subcommand_tokens...`.
fn matches_binary_subcommand(tokens: &[&str], binary: &str, subcmds: &[String]) -> bool {
    let mut found_binary = false;
    let mut skip_next = false;
    let mut subcmd_idx = 0;

    for token in tokens {
        if skip_next {
            skip_next = false;
            continue;
        }
        if !found_binary {
            if *token == binary {
                if subcmds.is_empty() {
                    return true;
                }
                found_binary = true;
                subcmd_idx = 0;
            }
            continue;
        }
        // After finding binary, skip flags
        if token.starts_with('-') {
            if *token == "-c" || *token == "-C" || *token == "--git-dir" || *token == "--work-tree"
            {
                skip_next = true;
            }
            continue;
        }
        // Match subcommand tokens in order
        if subcmd_idx < subcmds.len() && *token == subcmds[subcmd_idx] {
            subcmd_idx += 1;
            if subcmd_idx == subcmds.len() {
                return true;
            }
        } else {
            found_binary = false;
        }
    }
    false
}

/// Detect patterns inside subshells: `$(...)` and backtick expressions.
/// Input should already be lowercased.
fn contains_in_subshell_lowered(lower: &str, patterns: &[(String, Vec<String>)]) -> bool {
    let mut pos = 0;
    while let Some(start) = lower[pos..].find("$(") {
        let abs_start = pos + start + 2;
        if let Some(end) = lower[abs_start..].find(')') {
            let inside = &lower[abs_start..abs_start + end];
            if command_matches_parsed(inside, patterns) {
                return true;
            }
            pos = abs_start + end + 1;
        } else {
            break;
        }
    }

    let mut pos = 0;
    while let Some(start) = lower[pos..].find('`') {
        let abs_start = pos + start + 1;
        if let Some(end) = lower[abs_start..].find('`') {
            let inside = &lower[abs_start..abs_start + end];
            if command_matches_parsed(inside, patterns) {
                return true;
            }
            pos = abs_start + end + 1;
        } else {
            break;
        }
    }

    false
}

#[cfg(test)]
#[path = "command_match_tests.rs"]
mod tests;
