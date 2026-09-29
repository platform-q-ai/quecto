//! Task dependency validation (#2272): `Tasks._dependencies`, ported
//! literally. The use case reads the graph (every task's id and its stored
//! dependencies) and asks this pure policy whether a task may depend on the
//! given list.
//!
//! The task id is the caller's value, not only an integer: Python found the
//! task by it through SQLite's INTEGER affinity (`"3"`, `3.0` and `true`
//! all find a row), then compares it with each dependency by `==` and keys
//! its graph by it. So `3.0` is task 3 and `true` is task 1, as a Python
//! dict key, while the text `"3"` is a node of its own that no dependency
//! equals: Python stores such a self edge, and so does this port. Nodes are
//! keyed by [`Node`], which gives Python's `hash`/`==` classes for the
//! values a board holds.
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::python_value::python_equal;
use super::{BoardError, RefusalKind};

/// The most dependencies one task may name.
pub const DEPENDENCIES_MAX: usize = 100;

/// `not isinstance(dependencies, list) or len(dependencies) > 100`.
///
/// # Errors
/// `dependencies must be a bounded list`.
pub fn dependency_list(value: &Value) -> Result<&[Value], BoardError> {
    match value {
        Value::Array(entries) if entries.len() <= DEPENDENCIES_MAX => Ok(entries),
        _ => Err(BoardError::new(
            RefusalKind::Invalid,
            "dependencies must be a bounded list",
        )),
    }
}

/// The rest of `_dependencies` for `task_id` depending on `dependencies`
/// (already a bounded list), over `graph`, every task's id and its stored
/// dependencies as loaded (`json.loads(row['dependencies'])`).
///
/// # Errors
/// `invalid, missing or self dependencies` for an entry that is not an
/// `int`, names no task, or equals `task_id`; `cyclic dependencies` when
/// the new edges close a cycle.
pub fn validate_dependencies(
    task_id: &Value,
    dependencies: &[Value],
    graph: &[(i64, Value)],
) -> Result<(), BoardError> {
    debug_assert!(dependencies.len() <= DEPENDENCIES_MAX, "a bounded list");
    let ids: HashSet<Node> = graph.iter().map(|(id, _)| Node::Integer(*id)).collect();
    let valid = |dependency: &Value| {
        integer(dependency)
            && ids.contains(&Node::of(dependency))
            && !python_equal(dependency, task_id)
    };
    if dependencies.iter().all(valid) {
        // `graph[task_id] = dependencies`: a task id equal to a stored one
        // replaces its edges.
        let mut edges: HashMap<Node, &[Value]> = graph
            .iter()
            .map(|(id, stored)| (Node::Integer(*id), children(stored)))
            .collect();
        edges.insert(Node::of(task_id), dependencies);
        acyclic(Node::of(task_id), &edges)
    } else {
        Err(BoardError::new(
            RefusalKind::Invalid,
            "invalid, missing or self dependencies",
        ))
    }
}

/// The literal depth-first search: an explicit stack of `(node, leaving)`,
/// a node met again while still active closes a cycle.
fn acyclic(start: Node, edges: &HashMap<Node, &[Value]>) -> Result<(), BoardError> {
    let mut visited = HashSet::new();
    let mut active = HashSet::new();
    let mut stack = vec![(start, false)];
    while let Some((node, leaving)) = stack.pop() {
        if leaving {
            active.remove(&node);
            visited.insert(node);
        } else if active.contains(&node) {
            return Err(BoardError::new(
                RefusalKind::DependencyCycle,
                "cyclic dependencies",
            ));
        } else if !visited.contains(&node) {
            active.insert(node.clone());
            let next = edges.get(&node).copied().unwrap_or(&[]);
            stack.push((node, true));
            stack.extend(next.iter().map(|child| (Node::of(child), false)));
        }
    }
    debug_assert!(active.is_empty(), "every entered node was left");
    Ok(())
}

/// `type(value) is int`: a JSON integer, never a boolean or a float.
fn integer(value: &Value) -> bool {
    matches!(value, Value::Number(number) if number.is_i64() || number.is_u64())
}

/// A stored dependency list; anything else (only a file edited outside the
/// board holds it) has no children here.
fn children(stored: &Value) -> &[Value] {
    stored.as_array().map_or(&[], Vec::as_slice)
}

/// A graph node as a Python dict key: numbers (booleans included) that are
/// equal are one key (`1 == 1.0 == True`), text is its own, and any other
/// value is keyed by its JSON text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Node {
    Integer(i64),
    Text(String),
    Other(String),
}

impl Node {
    fn of(value: &Value) -> Self {
        match value {
            Value::Bool(flag) => Self::Integer(i64::from(*flag)),
            // An integer beyond i64 names no task and is no task id the
            // board can bind, so it keys a node of its own by its JSON text
            // (as does a float no i64 equals); either way it has no edges.
            Value::Number(number) => number
                .as_i64()
                .or_else(|| number.as_f64().and_then(integral))
                .map_or_else(|| Self::Other(value.to_string()), Self::Integer),
            Value::String(text) => Self::Text(text.clone()),
            Value::Null | Value::Array(_) | Value::Object(_) => Self::Other(value.to_string()),
        }
    }
}

/// The integer an integral float equals, within the range a task id holds.
fn integral(float: f64) -> Option<i64> {
    const BOUND: f64 = 9_223_372_036_854_775_808.0;
    let exact = float.is_finite() && float.fract() == 0.0 && (-BOUND..BOUND).contains(&float);
    exact.then_some(float as i64)
}

#[cfg(test)]
#[path = "dependencies_tests.rs"]
mod tests;
