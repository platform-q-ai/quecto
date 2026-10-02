//! #2414: the `agents.defaults` keys the watermark context removed (the
//! old pruning rules and the switch between the two modes), as pure policy
//! over a configuration's JSON document, beside the overlay policy's own
//! key rules: which of them a document sets, and the document without them.

use serde_json::{Map, Value};

/// The removed keys, under `agents.defaults`, in the order they are named.
pub const REMOVED_DEFAULTS_KEYS: [&str; 6] = [
    "context_mode",
    "context_collapse_after_tool_calls",
    // The pre-#1017 name of the tool dial.
    "context_collapse_after_turns",
    "context_collapse_after_messages",
    "context_collapse_large_result_tokens",
    "context_collapse_large_result_after_turns",
];

/// Why the keys went, for every message that names them.
pub const WHY_REMOVED: &str = "removed in #2414: the watermark context is the only context mode \
    (the context only grows at its end and is cut once at context_high_tokens down to \
    context_low_tokens)";

fn defaults(document: &Map<String, Value>) -> Option<&Map<String, Value>> {
    document.get("agents")?.get("defaults")?.as_object()
}

/// The removed keys `document` sets (to anything, `null` included), in
/// [`REMOVED_DEFAULTS_KEYS`] order.
pub fn removed_keys_set(document: &Map<String, Value>) -> Vec<&'static str> {
    let Some(defaults) = defaults(document) else {
        return Vec::new();
    };
    REMOVED_DEFAULTS_KEYS
        .into_iter()
        .filter(|key| defaults.contains_key(*key))
        .collect()
}

/// `document` without the removed keys, for checking everything else.
pub fn without_removed_keys(document: &Map<String, Value>) -> Map<String, Value> {
    let mut stripped = document.clone();
    let defaults = stripped
        .get_mut("agents")
        .and_then(|agents| agents.get_mut("defaults"))
        .and_then(Value::as_object_mut);
    if let Some(defaults) = defaults {
        defaults.retain(|key, _| !REMOVED_DEFAULTS_KEYS.contains(&key.as_str()));
    }
    debug_assert!(removed_keys_set(&stripped).is_empty());
    stripped
}

#[cfg(test)]
#[path = "removed_keys_tests.rs"]
mod tests;
