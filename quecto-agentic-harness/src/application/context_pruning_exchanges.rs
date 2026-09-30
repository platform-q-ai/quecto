// #2349 review M1: removal keeps call/result exchanges whole. An assistant
// message with tool calls and every result of those calls go or stay
// together: a provider rejects a call without its result, or a result
// without its call (OpenAI chat completions with a 400). Pure policy over
// the message list.

use super::{estimate_message_tokens, estimate_total_tokens};
use crate::domain::message::{Message, Role, ToolCall};
use std::collections::{BTreeMap, BTreeSet};

/// Per message, the exchange it belongs to: the index of the assistant
/// message whose call it answers (the assistant's own index for itself), or
/// its own index when it belongs to no exchange.
pub(super) fn exchange_groups(messages: &[Message]) -> Vec<usize> {
    let mut callers: BTreeMap<&str, usize> = BTreeMap::new();
    let mut groups = Vec::with_capacity(messages.len());
    for (index, msg) in messages.iter().enumerate() {
        let caller = match (&msg.role, msg.tool_call_id.as_deref()) {
            (Role::Tool, Some(id)) => callers.get(id).copied(),
            _ => None,
        };
        if msg.role == Role::Assistant {
            for call in &msg.tool_calls {
                callers.insert(call.id.as_str(), index);
            }
        }
        groups.push(caller.unwrap_or(index));
    }
    debug_assert!(groups.iter().enumerate().all(|(i, &g)| g <= i));
    groups
}

/// Widen `exempt` over whole exchanges: a message kept keeps every other
/// message of its exchange (a pinned caller keeps all of its results, a
/// pinned result keeps its caller and siblings).
pub(super) fn keep_exchanges_whole(exempt: &mut [bool], groups: &[usize]) {
    debug_assert_eq!(exempt.len(), groups.len());
    let kept: BTreeSet<usize> = groups
        .iter()
        .zip(exempt.iter())
        .filter(|&(_, &keep)| keep)
        .map(|(&group, _)| group)
        .collect();
    for (keep, group) in exempt.iter_mut().zip(groups) {
        *keep |= kept.contains(group);
    }
}

/// Remove whole non-exempt exchanges, oldest first, until the total fits
/// `max_tokens`, in a single pass. Never stops inside an exchange. Returns the
/// number of messages removed and the calls they carried, moved out (#2348).
/// `exempt` must already be whole over exchanges ([`keep_exchanges_whole`]).
pub(super) fn drop_exchanges_until_under_budget(
    messages: &mut Vec<Message>,
    max_tokens: usize,
    exempt: &[bool],
    groups: &[usize],
) -> (usize, Vec<ToolCall>) {
    let mut units: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, (&keep, &group)) in exempt.iter().zip(groups).enumerate() {
        if !keep {
            units.entry(group).or_default().push(index);
        }
    }
    let mut total = estimate_total_tokens(messages);
    let mut dropping = vec![false; messages.len()];
    for unit in units.values() {
        if total <= max_tokens {
            break;
        }
        for &index in unit {
            total = total.saturating_sub(estimate_message_tokens(&messages[index]));
            dropping[index] = true;
        }
    }
    let dropped = dropping.iter().filter(|&&drop| drop).count();
    let mut calls = Vec::new();
    let kept = Vec::with_capacity(messages.len() - dropped);
    for (msg, drop) in std::mem::replace(messages, kept).into_iter().zip(dropping) {
        match drop {
            true => calls.extend(msg.tool_calls),
            false => messages.push(msg),
        }
    }
    (dropped, calls)
}
