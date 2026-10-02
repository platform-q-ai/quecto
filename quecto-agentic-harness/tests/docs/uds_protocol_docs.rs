//! The UDS protocol reference (`docs/uds-protocol.md`) has a section for
//! every command type the agent accepts. A swarm audit (2026-10-02) found
//! `delete_all_subagents` and `refresh_models` accepted but undocumented;
//! this keeps the reference whole. `get_messages_tail` stays out on purpose.
use crate::common::read_repo_file;

/// The command type names `AgentCommand::type_name` answers, read from its
/// match arms (`Self::Variant { .. } => "name",`).
fn accepted_command_types() -> Vec<String> {
    let source = read_repo_file("src/interface/cli/protocol_commands.rs");
    let start = source
        .find("pub fn type_name(&self)")
        .expect("AgentCommand::type_name exists");
    let body = &source[start..];
    let end = body.find("\n    }\n").expect("type_name has a body");
    body[..end]
        .lines()
        .filter_map(|line| {
            let (_, rest) = line.split_once("=> \"")?;
            let (name, _) = rest.split_once('"')?;
            Some(name.to_string())
        })
        .collect()
}

/// Accepted but deliberately undocumented: `get_messages_tail` is a
/// deprecated alias for `get_messages` with `count`, kept out of the docs on
/// purpose (see `repo_docs::agent_cmd_docs_match_tool_schema`).
const UNDOCUMENTED_ALIASES: &[&str] = &["get_messages_tail"];

#[test]
fn every_accepted_command_type_has_a_section() {
    let doc = read_repo_file("docs/uds-protocol.md");
    let headings: Vec<&str> = doc.lines().filter(|l| l.starts_with("### ")).collect();
    let commands = accepted_command_types();
    assert!(commands.len() >= 30, "read the command list: {commands:?}");
    let missing: Vec<&String> = commands
        .iter()
        .filter(|name| !UNDOCUMENTED_ALIASES.contains(&name.as_str()))
        .filter(|name| {
            let tag = format!("`{name}`");
            !headings.iter().any(|heading| heading.contains(&tag))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "docs/uds-protocol.md has no section for {missing:?}"
    );
}
