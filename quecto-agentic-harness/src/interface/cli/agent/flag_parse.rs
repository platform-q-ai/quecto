//! Value-parsing helpers for the `agent` subcommand flag parser.

/// Return `args[i+1]` or push `err_msg` to stderr and return `None`.
pub(super) fn next_arg<'a>(
    args: &'a [String],
    i: usize,
    err_msg: &str,
    stderr: &mut String,
) -> Option<&'a str> {
    if i + 1 < args.len() {
        Some(args[i + 1].as_str())
    } else {
        stderr.push_str(&format!("agent: {err_msg}\n"));
        None
    }
}

/// Parse a positive non-zero u32 for `--max-iterations`.
pub(super) fn parse_pos_u32(val: &str, flag: &str, stderr: &mut String) -> Option<u32> {
    match val.parse::<u32>() {
        Ok(n) if n > 0 => Some(n),
        _ => {
            stderr.push_str(&format!("agent: {flag} requires a positive integer\n"));
            None
        }
    }
}

/// Parse a positive non-zero u64 for `--max-time`.
pub(super) fn parse_pos_u64(val: &str, flag: &str, stderr: &mut String) -> Option<u64> {
    match val.parse::<u64>() {
        Ok(n) if n > 0 => Some(n),
        _ => {
            stderr.push_str(&format!("agent: {flag} requires a positive integer\n"));
            None
        }
    }
}

/// Parse an effort level string, writing an error to `stderr` on failure.
pub(super) fn parse_effort_level(
    val: &str,
    stderr: &mut String,
) -> Option<crate::domain::provider::EffortLevel> {
    crate::domain::provider::EffortLevel::parse(val).or_else(|| {
        stderr.push_str(&format!(
            "agent: invalid effort level '{}'; expected one of: low, medium, high, max\n",
            val
        ));
        None
    })
}

/// Parse and validate a session name from `args[i+1]`.
pub(super) fn parse_session_name(args: &[String], i: usize, stderr: &mut String) -> Option<String> {
    let name = next_arg(args, i, "-s requires a session name", stderr)?;
    if !crate::interface::cli::is_valid_session_name(name) {
        stderr.push_str("agent: session name must contain only alphanumeric, '-', or '_'\n");
        return None;
    }
    Some(name.to_string())
}

/// Parse the `--mode` flag value. Returns `Some(true)` for `"uds"`, `None`
/// (with an error written to `stderr`) for any unknown value.
pub(super) fn parse_agent_mode(val: &str, stderr: &mut String) -> Option<bool> {
    match val {
        "uds" => Some(true),
        other => {
            stderr.push_str(&format!(
                "agent: --mode '{other}' is not valid; supported: uds\n"
            ));
            None
        }
    }
}
