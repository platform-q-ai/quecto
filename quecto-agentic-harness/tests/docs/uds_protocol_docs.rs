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

/// The `### `name`` section of the reference: from its heading to the next.
fn command_section<'a>(doc: &'a str, name: &str) -> &'a str {
    let heading = format!("### `{name}`");
    let start = doc
        .find(&heading)
        .unwrap_or_else(|| panic!("no section for {name}"));
    let body = &doc[start + heading.len()..];
    &body[..body.find("\n### ").unwrap_or(body.len())]
}

/// #2422: `prompt`, `steer` and `follow_up` document their `images` field,
/// and the reference quotes every refusal exactly as `quecto_image` words it.
#[test]
fn image_attachments_are_documented_with_their_exact_refusals() {
    use quecto_image::{ImageMime, ImageRefusal, ImagesRefusal};
    let doc = read_repo_file("docs/uds-protocol.md");
    for name in ["prompt", "steer", "follow_up"] {
        assert!(
            command_section(&doc, name).contains("| `images` |"),
            "the {name} section has no `images` row"
        );
    }
    let image = |index, refusal| ImagesRefusal::Image { index, refusal }.to_string();
    let refusals = [
        image(1, ImageRefusal::UnsupportedMime("image/svg+xml".into())),
        image(0, ImageRefusal::InvalidBase64),
        image(0, ImageRefusal::SignatureMismatch(ImageMime::Png)),
        image(0, ImageRefusal::TooLarge),
        image(0, ImageRefusal::Unreadable(ImageMime::Png)),
        ImagesRefusal::TooMany(9).to_string(),
    ];
    let missing: Vec<&String> = refusals
        .iter()
        .filter(|text| !doc.contains(&format!("`{text}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/uds-protocol.md does not quote {missing:?}"
    );
}
