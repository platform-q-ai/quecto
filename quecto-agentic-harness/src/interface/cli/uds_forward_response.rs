/// Validate and unwrap a forwarded child's command response.
pub(super) fn parse_forwarded_response(
    line: &str,
    command: &str,
) -> Result<serde_json::Value, String> {
    let parsed: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    if parsed.get("success").and_then(|value| value.as_bool()) != Some(true) {
        return Err(parsed
            .get("error")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{command} failed")));
    }
    if parsed.get("command").and_then(|value| value.as_str()) != Some(command) {
        return Err("unexpected child response command".into());
    }
    parsed
        .get("data")
        .cloned()
        .ok_or_else(|| format!("{command} response missing data"))
}

/// Validate and unwrap a forwarded child's `get_message` response.
pub(super) fn parse_forwarded_get_message(line: &str) -> Result<serde_json::Value, String> {
    parse_forwarded_response(line, "get_message")
}

/// Validate and unwrap a forwarded child's `get_messages` response.
pub(super) fn parse_forwarded_get_messages(line: &str) -> Result<serde_json::Value, String> {
    parse_forwarded_response(line, "get_messages")
}

#[cfg(test)]
#[path = "uds_forward_response_cov_tests.rs"]
mod cov_tests;
