//! The stable ids a persisted `tools.policy` entry may name (#2217).
//!
//! A policy entry is keyed by a tool's stable id. One this entrypoint
//! registers no tool for is kept and ignored, which is right for a bundled
//! tool another entrypoint builds and for a tool a release retired, but
//! hides a typo: a restriction that never applies. This catalogue tells
//! them apart by the namespace the id was minted in: a UDS extension's or a
//! runtime tool's entry awaits its registration; a bundled-native id is
//! checked against the one list of bundled tools (a drift test builds every
//! entrypoint's registry and checks each bundled registration is here).

use super::tool_descriptor::ToolSource;
use super::tool_id::{parse_stable_tool_id, stable_tool_id};

/// Every bundled tool any entrypoint may build: `(provider id, name)`.
pub const BUNDLED_TOOLS: &[(&str, &str)] = &[
    ("quecto:official-tools", "bash"),
    ("quecto:official-tools", "docs"),
    ("quecto:official-tools", "edit"),
    ("quecto:official-tools", "find"),
    ("quecto:official-tools", "grep"),
    ("quecto:official-tools", "ls"),
    ("quecto:official-tools", "read"),
    ("quecto:official-tools", "swarm"),
    ("quecto:official-tools", "write"),
    ("quecto:session-tools", "recall"),
    ("quecto:agent-control", "agent_cmd"),
    ("quecto:agent-control", "spawn"),
    ("quecto:workflow", "workflow"),
    ("web", "web_fetch"),
    ("web", "web_search"),
];

/// Bundled tools a release removed: `(provider id, name, removed by)`.
/// A policy entry naming one is dead and safe to delete.
pub const RETIRED_TOOLS: &[(&str, &str, &str)] =
    &[("quecto:official-tools", "python_lab", "#1684")];

/// What a policy entry that matched no registered tool names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmatchedPolicyEntry {
    /// A bundled tool this entrypoint does not build: kept for the others.
    BundledElsewhere,
    /// A tool a release removed: the entry is dead.
    Retired { removed_by: &'static str },
    /// A tool of a namespace that registers after start-up (a UDS
    /// extension, a runtime tool): the entry applies when it registers.
    AwaitingRegistration { source: ToolSource },
    /// A bundled-native id no bundled tool has, current or retired, or no
    /// stable id at all: most likely a typo.
    Unknown,
}

/// The stable id a bundled tool registers under.
pub fn bundled_stable_id(provider_id: &str, name: &str) -> String {
    stable_tool_id(ToolSource::BundledNative, provider_id, name)
}

/// Classify a policy entry that matched no registered tool, by the
/// namespace its stable id was minted in (#2247 round 2 L2).
pub fn classify_unmatched_policy_entry(stable_id: &str) -> UnmatchedPolicyEntry {
    match parse_stable_tool_id(stable_id).map(|parsed| parsed.source) {
        Some(ToolSource::BundledNative) => classify_bundled(stable_id),
        Some(source @ (ToolSource::Uds | ToolSource::Runtime)) => {
            UnmatchedPolicyEntry::AwaitingRegistration { source }
        }
        None => UnmatchedPolicyEntry::Unknown,
    }
}

/// The warning for one unknown entry (a typo), without a severity prefix:
/// every entrypoint and a reload's reply word it the same. The key is shown
/// bounded and escaped ([`shown_entry_id`]).
pub fn unknown_policy_entry_warning(stable_id: &str) -> String {
    format!(
        "tools.policy: no tool has stable id '{}', so its entry never applies; fix or remove it under tools.policy.entries",
        shown_entry_id(stable_id)
    )
}

/// The most bytes of an entry key [`shown_entry_id`] echoes.
pub const SHOWN_ENTRY_ID_MAX_BYTES: usize = 128;

/// A policy entry key as a message or a log may echo it (#2247 review): the
/// key is whatever a file or a key path supplied. Printable ASCII is shown
/// as is; every other character is escaped (`\n`, `\u{1b}`), so nothing
/// reaches a terminal raw; the result is cut at
/// [`SHOWN_ENTRY_ID_MAX_BYTES`] on a character boundary, marked `…`.
pub fn shown_entry_id(raw: &str) -> String {
    let mut shown = String::new();
    for ch in raw.chars() {
        let piece = match ch {
            ' '..='~' => ch.to_string(),
            other => other.escape_default().to_string(),
        };
        if shown.len() + piece.len() > SHOWN_ENTRY_ID_MAX_BYTES {
            shown.push('…');
            return shown;
        }
        shown.push_str(&piece);
    }
    debug_assert!(shown.len() <= SHOWN_ENTRY_ID_MAX_BYTES);
    shown
}

/// A bundled-native id: current, retired, or neither (a typo).
fn classify_bundled(stable_id: &str) -> UnmatchedPolicyEntry {
    let bundled = BUNDLED_TOOLS
        .iter()
        .any(|(provider_id, name)| bundled_stable_id(provider_id, name) == stable_id);
    if bundled {
        return UnmatchedPolicyEntry::BundledElsewhere;
    }
    RETIRED_TOOLS
        .iter()
        .find(|(provider_id, name, _)| bundled_stable_id(provider_id, name) == stable_id)
        .map_or(UnmatchedPolicyEntry::Unknown, |(_, _, removed_by)| {
            UnmatchedPolicyEntry::Retired { removed_by }
        })
}

#[cfg(test)]
#[path = "tool_policy_catalogue_tests.rs"]
mod tests;
