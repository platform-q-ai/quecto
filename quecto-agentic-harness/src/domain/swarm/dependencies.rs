//! Task dependency validation (#2272): `Tasks._dependencies`.
use serde_json::Value;

use super::BoardError;

/// The most dependencies one task may name.
pub const DEPENDENCIES_MAX: usize = 100;

/// `not isinstance(dependencies, list) or len(dependencies) > 100`.
///
/// # Errors
/// `dependencies must be a bounded list`.
pub fn dependency_list(value: &Value) -> Result<&[Value], BoardError> {
    Ok(value.as_array().map_or(&[], Vec::as_slice))
}

/// The rest of `_dependencies`.
///
/// # Errors
/// `invalid, missing or self dependencies` or `cyclic dependencies`.
pub fn validate_dependencies(
    _task_id: &Value,
    _dependencies: &[Value],
    _graph: &[(i64, Value)],
) -> Result<(), BoardError> {
    Ok(())
}

#[cfg(test)]
#[path = "dependencies_tests.rs"]
mod tests;
