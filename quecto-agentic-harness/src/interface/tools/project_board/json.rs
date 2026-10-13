//! JSON text read as a request: like `serde_json`, except that an object
//! naming one key twice is refused (`serde_json` keeps the last silently,
//! so `{"to": "ready", "to": "done"}` would mean whichever came last).
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::cell::Cell;

use crate::domain::project_board::value_objects::telemetry::FaultKind;
use std::fmt;

/// Why text is no request: its reader's words, and whether it named a key
/// twice (otherwise it is not JSON), which a call's record tells apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictError {
    pub message: String,
    pub duplicate_key: bool,
}

impl StrictError {
    /// How the text was wrong, as a call's record holds it.
    pub fn fault(&self) -> FaultKind {
        match self.duplicate_key {
            true => FaultKind::DuplicateKey,
            false => FaultKind::NotJson,
        }
    }
}

impl fmt::Display for StrictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// The value `text` holds.
///
/// # Errors
/// Not JSON, or an object naming a key twice.
pub fn strict(text: &str) -> Result<Value, StrictError> {
    let duplicate = Cell::new(false);
    let refused = |error: serde_json::Error| StrictError {
        message: error.to_string(),
        duplicate_key: duplicate.get(),
    };
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = Strict(&duplicate)
        .deserialize(&mut deserializer)
        .map_err(refused)?;
    deserializer.end().map_err(refused)?;
    Ok(value)
}

/// Reads a value strictly, setting its flag when an object names a key
/// twice.
struct Strict<'a>(&'a Cell<bool>);

impl<'de> de::DeserializeSeed<'de> for Strict<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Strict<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(Strict(self.0))? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value_seed(Strict(self.0))?;
            if object.insert(key.clone(), value).is_some() {
                self.0.set(true);
                return Err(de::Error::custom(format!("the key {key:?} is given twice")));
            }
        }
        Ok(Value::Object(object))
    }
}
