//! The argv of a `claude -p` stream-json member (#2286): the flags, and
//! each value from the launch spec, checked before anything is spawned.

use serde_json::Value;

use crate::application::external_agent::dto::{ExternalAgentLaunchError, ExternalAgentLaunchSpec};

/// Every flag `claude` is given: the only arguments telemetry names (a
/// flag's value may be inline JSON).
const CLAUDE_FLAGS: &[&str] = &[
    "-p",
    "--input-format",
    "--output-format",
    "--verbose",
    "--model",
    "--tools",
    "--mcp-config",
    "--strict-mcp-config",
    "--settings",
    "--setting-sources",
    "--permission-mode",
    "--allow-dangerously-skip-permissions",
    "--no-session-persistence",
    "--max-budget-usd",
];

/// The flags of `arguments`, without their values.
pub(super) fn flags_of(arguments: &[String]) -> Vec<&str> {
    arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| CLAUDE_FLAGS.contains(argument))
        .collect()
}

/// A model name: letters, digits and `-._[]`, not starting with `-`.
fn valid_model(model: &str) -> bool {
    model
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._[]".contains(c))
}

/// A tool name: letters, digits and `_`, starting with a letter.
fn valid_tool(tool: &str) -> bool {
    tool.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn json_object(flag: &str, value: &Value) -> Result<String, ExternalAgentLaunchError> {
    match value {
        Value::Object(_) => Ok(value.to_string()),
        _ => Err(invalid(format!("{flag} must be a JSON object"))),
    }
}

/// The argv after the program: the stream-json flags and the spec's values,
/// each value checked first.
///
/// `--mcp-config` and `--settings` are inline JSON, and argv is readable
/// by every local user through procfs. Nothing secret may be inlined: the
/// credential reaches `claude` only through its environment, and S5 must
/// hand bridge tokens over by a file in the member's private directory,
/// never in this JSON. An argv that would carry the credential's value is
/// refused.
pub fn claude_arguments(
    spec: &ExternalAgentLaunchSpec,
) -> Result<Vec<String>, ExternalAgentLaunchError> {
    if !valid_model(&spec.model) {
        return Err(invalid(format!(
            "model {:?} is not a model name",
            spec.model
        )));
    }
    if let Some(tool) = spec.tools.iter().find(|tool| !valid_tool(tool)) {
        return Err(invalid(format!("tool {tool:?} is not a tool name")));
    }
    if !(spec.max_budget_usd.is_finite() && spec.max_budget_usd > 0.0) {
        return Err(invalid(format!(
            "the budget {} is not a positive amount",
            spec.max_budget_usd
        )));
    }
    let mcp_config = json_object("--mcp-config", &spec.mcp_config)?;
    let settings = json_object("--settings", &spec.settings)?;
    let arguments: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        &spec.model,
        "--tools",
        &spec.tools.join(","),
        "--mcp-config",
        &mcp_config,
        "--strict-mcp-config",
        "--settings",
        &settings,
        "--setting-sources",
        "project",
        "--permission-mode",
        "bypassPermissions",
        // Spike #2264's shim passed this whenever the mode was
        // bypassPermissions. Claude 2.1.280 honoured the mode without it
        // (round-1 live check), but a version that requires the opt-in
        // would silently fall back to prompting: match the spike.
        "--allow-dangerously-skip-permissions",
        "--no-session-persistence",
        "--max-budget-usd",
        &spec.max_budget_usd.to_string(),
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect();
    debug_assert!(
        arguments
            .iter()
            .filter(|argument| argument.starts_with('-'))
            .all(|flag| CLAUDE_FLAGS.contains(&flag.as_str())),
        "every flag is one telemetry may name"
    );
    let secret = spec.credential.value.as_str();
    let argv_clean =
        secret.is_empty() || arguments.iter().all(|argument| !argument.contains(secret));
    if argv_clean {
        return Ok(arguments);
    }
    Err(invalid(format!(
        "the argv would carry the value of {}: argv is readable by every local user",
        spec.credential.name
    )))
}

fn invalid(detail: String) -> ExternalAgentLaunchError {
    ExternalAgentLaunchError::InvalidSpec(detail)
}
