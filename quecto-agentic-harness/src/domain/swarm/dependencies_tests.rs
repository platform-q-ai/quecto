//! `_dependencies`, messages asserted by string.
use serde_json::{Value, json};

use super::*;

const BOUNDED: &str = "dependencies must be a bounded list";
const INVALID: &str = "invalid, missing or self dependencies";
const CYCLIC: &str = "cyclic dependencies";

/// Tasks 1 → [], 2 → [1], 3 → [2], 4 → [] as the store holds them.
fn graph() -> Vec<(i64, Value)> {
    vec![
        (1, json!([])),
        (2, json!([1])),
        (3, json!([2])),
        (4, json!([])),
    ]
}

fn check(task_id: Value, dependencies: Value) -> Result<(), String> {
    let list = dependency_list(&dependencies).map_err(|refusal| refusal.0)?;
    validate_dependencies(&task_id, list, &graph()).map_err(|refusal| refusal.0)
}

#[test]
fn dependencies_reject_missing_self_and_cycles() {
    let too_many: Vec<i64> = std::iter::repeat_n(1, DEPENDENCIES_MAX + 1).collect();
    let most: Vec<i64> = std::iter::repeat_n(1, DEPENDENCIES_MAX).collect();
    let cases: [(Value, Value, Result<(), &str>); 17] = [
        (json!(4), json!([1, 2, 3]), Ok(())),
        (json!(4), json!([]), Ok(())),
        (json!(4), json!([1, 1]), Ok(())),
        (json!(4), json!(most), Ok(())),
        (json!(4), json!(too_many), Err(BOUNDED)),
        (json!(4), json!(null), Err(BOUNDED)),
        (json!(4), json!({"1": 1}), Err(BOUNDED)),
        (json!(4), json!("1"), Err(BOUNDED)),
        // A missing id, a self edge, and ids that are not `int`.
        (json!(4), json!([99_999]), Err(INVALID)),
        (json!(4), json!([4]), Err(INVALID)),
        (json!(4), json!([true]), Err(INVALID)),
        (json!(4), json!([1.0]), Err(INVALID)),
        (json!(4), json!(["1"]), Err(INVALID)),
        (json!(4), json!([u64::MAX]), Err(INVALID)),
        // Task 1 depending on 3 closes 1 → 3 → 2 → 1.
        (json!(1), json!([3]), Err(CYCLIC)),
        (json!(1), json!([2]), Err(CYCLIC)),
        (json!(2), json!([3]), Err(CYCLIC)),
    ];
    for (task_id, dependencies, expected) in cases {
        assert_eq!(
            check(task_id.clone(), dependencies.clone()),
            expected.map_err(str::to_owned),
            "{task_id} -> {dependencies}"
        );
    }
}

/// The task id is the caller's value: Python compares it with each
/// dependency by `==` and keys the graph by it, so `3.0` and `true` are
/// tasks 3 and 1, while the text `"3"` is a node of its own, which no
/// dependency equals.
#[test]
fn a_loose_task_id_is_compared_and_keyed_as_python_does() {
    assert_eq!(check(json!(3.0), json!([3])), Err(INVALID.to_owned()));
    assert_eq!(check(json!(true), json!([1])), Err(INVALID.to_owned()));
    // `graph[True] = [3]` replaces task 1's dependencies: 1 → 3 → 2 → 1.
    assert_eq!(check(json!(true), json!([3])), Err(CYCLIC.to_owned()));
    assert_eq!(check(json!(1.0), json!([3])), Err(CYCLIC.to_owned()));
    // `graph["3"]` is a new node: task 3 itself is not seen as a self
    // edge, and 3 → 2 → 1 has no cycle.
    assert_eq!(check(json!("3"), json!([3])), Ok(()));
    assert_eq!(check(json!("1"), json!([3])), Ok(()));
}

/// A node the graph does not hold has no children (`graph.get(node, [])`),
/// and stored dependencies that are not a list are none either.
#[test]
fn nodes_without_a_list_have_no_children() {
    let graph = vec![(1, json!([7])), (2, json!("not a list")), (3, json!([2]))];
    assert_eq!(
        validate_dependencies(&json!(9), &[json!(1)], &graph),
        Ok(())
    );
    assert_eq!(
        validate_dependencies(&json!(9), &[json!(3)], &graph),
        Ok(())
    );
    assert_eq!(
        validate_dependencies(&json!(1), &[json!(1)], &graph)
            .unwrap_err()
            .0,
        INVALID
    );
}

/// A diamond is not a cycle: a node reached twice is visited once.
#[test]
fn a_diamond_is_not_a_cycle() {
    let graph = vec![
        (1, json!([])),
        (2, json!([1])),
        (3, json!([1])),
        (4, json!([2, 3])),
    ];
    assert_eq!(
        validate_dependencies(&json!(5), &[json!(4), json!(2), json!(3)], &graph),
        Ok(())
    );
}
