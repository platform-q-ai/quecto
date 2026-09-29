//! Structured `swarm` board ops (#2279, epic #2265 D4). RED STUB.
use serde_json::Value;

/// One argument of a board op.
pub struct ArgSpec {
    pub name: &'static str,
    pub json_type: &'static str,
    pub required: bool,
    pub default: Option<&'static str>,
}

/// One board op.
pub struct OpSpec {
    pub name: &'static str,
    pub args: &'static [ArgSpec],
    pub read_only: bool,
}

pub const BOARD_OPS: &[OpSpec] = &[];
pub const READ_ONLY_OPS: &[&str] = &[];
pub const MUTATING_OPS: &[&str] = &[];

pub fn tool_schema() -> Value {
    Value::Null
}

/// # Errors
/// Never in the stub.
pub fn member_arguments(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|error| error.to_string())
}

pub fn wire_text(value: &Value) -> String {
    value.to_string()
}

#[cfg(test)]
#[path = "swarm_board_ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "swarm_board_ops_table_tests.rs"]
mod table_tests;
