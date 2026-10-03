//! Reading another process's slim projection, forward compatibly (#2210
//! review). A parent reads its children's busy `get_state` snapshots, and a
//! child may be a newer build (a running parent spawns the `quecto` on
//! `PATH`). The projection grows by additive members, so at every object
//! boundary this reader keeps the members this harness knows — read by the
//! closed types of `state_snapshot`, with their declared types — and drops
//! a member it does not know when its name has a member's shape (a camelCase
//! identifier): an additive member of a newer writer. A name of any other
//! shape is refused. What was read is what is relayed: the typed value,
//! re-serialized, never the child's bytes.
use serde_json::{Map, Value};

use super::{StateSnapshot, UnchangedSnapshot};

/// One object boundary of the projection: its members, and which of them
/// are objects or arrays of objects with boundaries of their own.
pub(super) struct Shape {
    pub(super) members: &'static [&'static str],
    objects: &'static [(&'static str, &'static Shape)],
    arrays: &'static [(&'static str, &'static Shape)],
}

const fn leaf(members: &'static [&'static str]) -> Shape {
    Shape {
        members,
        objects: &[],
        arrays: &[],
    }
}

pub(super) const PROGRESS: Shape = leaf(&["state", "reason"]);
pub(super) const TEMPLATE: Shape = leaf(&["id"]);
pub(super) const STEP: Shape = leaf(&["index", "key", "label", "phase", "done"]);
pub(super) const WORKFLOW: Shape = Shape {
    members: &["activeTemplate", "currentStep"],
    objects: &[("activeTemplate", &TEMPLATE), ("currentStep", &STEP)],
    arrays: &[],
};
pub(super) const RECEIPT: Shape = leaf(&["id", "command", "status"]);
pub(super) const COOLDOWN: Shape = leaf(&["state", "remainingSeconds"]);
pub(super) const GROUP: Shape = Shape {
    members: &["group", "cooldown", "lastRefusal"],
    objects: &[("cooldown", &COOLDOWN)],
    arrays: &[],
};
pub(super) const COUNTERS: Shape = leaf(&["completed", "refused", "cancelled", "abandoned"]);
pub(super) const ADMISSION: Shape = Shape {
    members: &[
        "waiting",
        "admitted",
        "longestWaitSeconds",
        "groups",
        "counters",
        "hidden",
        "revision",
        "directory",
        "epoch",
        "connected",
        "authorityStatus",
    ],
    objects: &[("counters", &COUNTERS)],
    arrays: &[("groups", &GROUP)],
};
pub(super) const WARNING: Shape = leaf(&["slot", "code", "message"]);
pub(super) const ATTEMPT: Shape = leaf(&[
    "number",
    "elapsedMs",
    "events",
    "outputBytes",
    "sinceLastEventMs",
    "firstTokenMs",
]);
pub(super) const MODEL_TURN: Shape = Shape {
    members: &["elapsedMs", "outputCapBytes", "attempt"],
    objects: &[("attempt", &ATTEMPT)],
    arrays: &[],
};
pub(super) const AGENT_REQUESTS: Shape =
    leaf(&["requests", "inputTokens", "cachedTokens", "outputTokens"]);
pub(super) const STATE: Shape = Shape {
    members: &[
        "state",
        "effort",
        "effortLevels",
        "model",
        "sessionKey",
        "progress",
        "generation",
        "workflow",
        "controlReceipts",
        "automaticTurnsSuspended",
        "repeatedFailureNotifications",
        "admission",
        "admissionWarnings",
        "modelTurn",
    ],
    objects: &[
        ("progress", &PROGRESS),
        ("workflow", &WORKFLOW),
        ("admission", &ADMISSION),
        ("modelTurn", &MODEL_TURN),
    ],
    arrays: &[
        ("controlReceipts", &RECEIPT),
        ("admissionWarnings", &WARNING),
    ],
};
pub(super) const UNCHANGED: Shape = Shape {
    members: &["unchanged", "generation", "modelTurn"],
    objects: &[("modelTurn", &MODEL_TURN)],
    arrays: &[],
};

/// Whether `name` has the shape of a member a newer writer may add: a
/// camelCase identifier, as every member of the projection is.
fn is_member_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

fn nested<'a>(within: &'a [(&str, &'a Shape)], name: &str) -> Option<&'a Shape> {
    within
        .iter()
        .find(|(member, _)| *member == name)
        .map(|(_, shape)| *shape)
}

/// `value` with only the members `shape` knows, at every boundary; `None`
/// when a member it does not know has no member's name. A value that is not
/// an object is kept as it is, for its type to judge.
pub(super) fn known(value: &Value, shape: &Shape) -> Option<Value> {
    let Some(object) = value.as_object() else {
        return Some(value.clone());
    };
    let mut kept = Map::new();
    for (name, member) in object {
        if shape.members.contains(&name.as_str()) {
            let member = match (
                nested(shape.objects, name),
                nested(shape.arrays, name),
                member.as_array(),
            ) {
                (Some(inner), _, _) => known(member, inner)?,
                (None, Some(inner), Some(items)) => Value::Array(
                    items
                        .iter()
                        .map(|item| known(item, inner))
                        .collect::<Option<_>>()?,
                ),
                _ => member.clone(),
            };
            kept.insert(name.clone(), member);
        } else if !is_member_name(name) {
            return None;
        }
    }
    Some(Value::Object(kept))
}

impl StateSnapshot {
    /// Read a slim projection written by this harness or a newer one: the
    /// members this harness knows, strictly, at every boundary; additive
    /// members of a newer writer dropped. The required members (`state`,
    /// `progress`, `model`, `generation`) still refuse any other shape, the
    /// full session state and the unchanged marker among them.
    pub fn read_forward_compatible(data: &Value) -> Option<Self> {
        serde_json::from_value(known(data, &STATE)?).ok()
    }
}

impl UnchangedSnapshot {
    /// Read the unchanged marker written by this harness or a newer one:
    /// `unchanged: true` and a `generation`, and the live measurements a
    /// `since` poll carries (`modelTurn`, read as `read_forward_compatible`
    /// reads it). An additive member of a newer writer is dropped as there;
    /// a member of the full projection is refused — a marker is not a
    /// projection.
    pub fn read(data: &Value) -> Option<Self> {
        let object = data.as_object()?;
        let projection_member = |name: &String| {
            STATE.members.contains(&name.as_str()) && !UNCHANGED.members.contains(&name.as_str())
        };
        if object.keys().any(projection_member) {
            return None;
        }
        let marker: Self = serde_json::from_value(known(data, &UNCHANGED)?).ok()?;
        marker.unchanged.then_some(marker)
    }
}
