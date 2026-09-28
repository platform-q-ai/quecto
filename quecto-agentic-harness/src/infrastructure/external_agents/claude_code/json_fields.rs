//! Reading one field of a stream-json object on its own (#2285): a field of
//! the wrong shape is absent, never the whole line's failure.

use serde_json::{Map, Value};

pub(super) type Object = Map<String, Value>;

/// A field decoded strictly: absent (or `null`), well formed, or present
/// with the wrong shape.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Strict<T> {
    Absent,
    Present(T),
    Malformed,
}

impl<T> Strict<T> {
    pub(super) fn of(value: Option<&Value>, read: impl Fn(&Value) -> Option<T>) -> Self {
        match value {
            None | Some(Value::Null) => Self::Absent,
            Some(value) => read(value).map_or(Self::Malformed, Self::Present),
        }
    }

    pub(super) fn present(self) -> Option<T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Absent | Self::Malformed => None,
        }
    }
}

pub(super) fn text(object: &Object, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(super) fn flag(object: &Object, key: &str) -> Option<bool> {
    object.get(key).and_then(Value::as_bool)
}

pub(super) fn number(object: &Object, key: &str) -> Option<f64> {
    object.get(key).and_then(Value::as_f64)
}

/// A non-negative count: an integer, or a finite non-negative float
/// rounded to the nearest one.
pub(super) fn count(object: &Object, key: &str) -> Option<u64> {
    object.get(key).and_then(count_of)
}

pub(super) fn count_of(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let float = value.as_f64()?.round();
        // In [0, 2^64) (NaN and infinities are not), so the cast is exact.
        (0.0..18_446_744_073_709_551_616.0)
            .contains(&float)
            .then_some(float as u64)
    })
}

/// Unix seconds: an integer, or a float truncated to whole seconds.
pub(super) fn seconds(object: &Object, key: &str) -> Option<i64> {
    let value = object.get(key)?;
    value.as_i64().or_else(|| {
        let float = value.as_f64()?.trunc();
        // In [-2^63, 2^63) (NaN and infinities are not), so the cast is exact.
        (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0)
            .contains(&float)
            .then_some(float as i64)
    })
}

pub(super) fn object<'a>(object: &'a Object, key: &str) -> Option<&'a Object> {
    object.get(key).and_then(Value::as_object)
}

/// A list field; anything else is an empty list.
pub(super) fn list<'a>(object: &'a Object, key: &str) -> &'a [Value] {
    object
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The strings of a list field; other entries are skipped.
pub(super) fn texts(object: &Object, key: &str) -> Vec<String> {
    list(object, key)
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
