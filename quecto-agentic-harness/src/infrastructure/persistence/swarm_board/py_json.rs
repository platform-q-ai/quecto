//! Python-compatible JSON text for the swarm board (#2268). RED stub.

use serde_json::Value;

/// `json.dumps(v, sort_keys=True, separators=(',', ':'))`.
pub fn encode(_value: &Value) -> String {
    String::new()
}

/// Plain `json.dumps(v)`.
pub fn dumps(_value: &Value) -> String {
    String::new()
}

/// Python `float.__repr__`, as `json.dumps` writes a float.
pub fn float_repr(_value: f64) -> String {
    String::new()
}

/// `json.loads`, through `serde_json::from_str`.
///
/// # Errors
/// Returns the parse error for text that is not JSON.
pub fn decode(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
#[path = "py_json_tests.rs"]
mod tests;
