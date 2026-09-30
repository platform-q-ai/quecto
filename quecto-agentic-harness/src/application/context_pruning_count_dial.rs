// #2213 / #2342: how many results a count dial collapses, and which. Split
// from `context_pruning_ceiling.rs` (decrease-only line ceiling). Pure
// policy over the message list.

use super::{in_flight_start, low_water};
use crate::application::context_pruning::snapshots::newest_snapshots;
use crate::domain::message::{Message, Role};

/// How many of `live` items a count dial collapses: none at or under
/// `limit`; once `limit` is crossed, enough to leave [`low_water`]`(limit)`.
pub fn count_to_collapse(live: usize, limit: usize) -> usize {
    if live > limit {
        live - low_water(limit)
    } else {
        0
    }
}

/// The tool results a count dial collapses (#2213), as `(to_collapse,
/// seen_end, collapsible)`: collapse `to_collapse` of the `collapsible`
/// results, oldest first, within `messages[..seen_end]`, which ends where
/// the in-flight exchange starts. Results never spilled (`spill_id ==
/// None`) would mint an unresolvable `recall()` stub, and the newest
/// snapshot of a state is its latest in full (#2342): neither is counted
/// nor collapsed.
pub fn tool_results_to_collapse(messages: &[Message], limit: usize) -> (usize, usize, Vec<bool>) {
    let newest = newest_snapshots(messages);
    let collapsible: Vec<bool> = messages
        .iter()
        .zip(&newest)
        .map(|(m, &newest)| {
            m.role == Role::Tool && !m.is_collapsed && m.spill_id.is_some() && !newest
        })
        .collect();
    let seen_end = in_flight_start(messages);
    let live = collapsible.iter().filter(|&&c| c).count();
    let seen = collapsible[..seen_end].iter().filter(|&&c| c).count();
    let to_collapse = count_to_collapse(live, limit).min(seen);
    debug_assert!(to_collapse <= seen && seen <= live);
    (to_collapse, seen_end, collapsible)
}
