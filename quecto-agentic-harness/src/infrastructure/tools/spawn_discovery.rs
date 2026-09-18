//! The roster line of the spawn tool's description (#2024 S4c): one line,
//! at most [`ROSTER_LINE_MAX_CHARS`] characters, naming the container
//! configs the launching agent can select — the `container: true` default
//! first, each with the layer that declared it — so a model knows the
//! menu before its first spawn call without a round trip. Presentation of
//! the environments capability's listing query; the query owns the rules
//! (default visibility while an overlay is withheld, ordering).

use crate::application::environments::dto::{ContainerConfigInventory, ContainerConfigLayer};
use crate::application::environments::use_cases::ListContainerConfigs;

pub const ROSTER_PREFIX: &str = "Available container configs: ";
pub const ROSTER_LINE_MAX_CHARS: usize = 120;
const WITHHELD_NOTE: &str = " (repo overlay untrusted — run quecto config trust)";

/// The line for a composed query; a query whose configuration cannot be
/// read still yields a line, naming the reason, so the model is never
/// shown a description that silently lacks the menu.
pub fn roster_line(query: Option<&ListContainerConfigs>) -> Option<String> {
    let query = query?;
    Some(match query.execute() {
        Ok(inventory) => format_roster_line(&inventory),
        Err(reason) => fit(format!("{ROSTER_PREFIX}none readable ({reason})"), ""),
    })
}

/// `Available container configs: <name> (default, repo-bound|global), …`
/// with the trailing entries folded into `+N more` when the line would
/// exceed the budget; the withheld-overlay note is never dropped.
pub fn format_roster_line(inventory: &ContainerConfigInventory) -> String {
    let note = if inventory.overlay_withheld {
        WITHHELD_NOTE
    } else {
        ""
    };
    if inventory.configs.is_empty() {
        return fit(format!("{ROSTER_PREFIX}none configured"), note);
    }
    let entries: Vec<String> = inventory
        .configs
        .iter()
        .map(|entry| {
            let layer = match entry.layer {
                ContainerConfigLayer::Overlay => "repo-bound",
                ContainerConfigLayer::Global => "global",
            };
            if entry.default {
                format!("{} (default, {layer})", entry.name)
            } else {
                format!("{} ({layer})", entry.name)
            }
        })
        .collect();
    let mut shown = entries.len();
    loop {
        let mut line = format!("{ROSTER_PREFIX}{}", entries[..shown].join(", "));
        let hidden = entries.len() - shown;
        if hidden > 0 {
            line.push_str(&format!(", +{hidden} more"));
        }
        line.push_str(note);
        line.push('.');
        if line.chars().count() <= ROSTER_LINE_MAX_CHARS || shown == 1 {
            return line;
        }
        shown -= 1;
    }
}

/// `body` + `note` + `.`, the body cut (with an ellipsis) when the whole
/// would exceed the budget.
fn fit(body: String, note: &str) -> String {
    let budget = ROSTER_LINE_MAX_CHARS.saturating_sub(note.chars().count() + 1);
    let body: String = if body.chars().count() > budget {
        let mut cut: String = body.chars().take(budget.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        body
    };
    format!("{body}{note}.")
}

#[cfg(test)]
#[path = "spawn_discovery_tests.rs"]
mod tests;
