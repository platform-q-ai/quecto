/// Validate and unwrap a forwarded child's `get_messages` response.
pub(super) fn parse_forwarded_get_messages(line: &str) -> Result<serde_json::Value, String> {
    let parsed: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    if parsed.get("success").and_then(|value| value.as_bool()) != Some(true) {
        return Err(parsed
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("get_messages failed")
            .to_string());
    }
    if parsed.get("command").and_then(|value| value.as_str()) != Some("get_messages") {
        return Err("unexpected child response command".into());
    }
    parsed
        .get("data")
        .cloned()
        .ok_or_else(|| "get_messages response missing data".into())
}
