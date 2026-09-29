//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! id source and codec stand-in, beside `board_test_support.rs` so each
//! file stays within its size budget.
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::Journal;
use crate::application::swarm::ports::{BoardEncoding, IdSource};
use crate::domain::swarm::BoardError;

/// `format(n, '032x')` for n = 1, 2, …, each draw journalled.
pub struct CounterIds {
    next: Mutex<u64>,
    journal: Journal,
}

impl CounterIds {
    pub fn journalling(journal: &Journal) -> Arc<Self> {
        Arc::new(Self {
            next: Mutex::new(1),
            journal: journal.clone(),
        })
    }
}

impl IdSource for CounterIds {
    fn hex32(&self) -> String {
        let mut next = self.next.lock().unwrap();
        let id = format!("{:032x}", *next);
        *next += 1;
        self.journal.lock().unwrap().push(format!("draw {id}"));
        id
    }
}

/// Stands in for the board codec: sorted keys, compact separators and
/// `ensure_ascii` escaping, as the real encoder writes them, so a size
/// bound counts the bytes the board stores (`é` is six). Floats keep
/// serde's text (`1e16`, where Python writes `1e+16`): the use cases only
/// measure the encoding, and no test here bounds a float-heavy value.
pub struct CompactEncoding;

impl BoardEncoding for CompactEncoding {
    fn encode(&self, value: &Value) -> Result<String, BoardError> {
        let compact = serde_json::to_string(&sorted(value)).unwrap();
        // Outside strings the text is ASCII; inside, each non-ASCII
        // character becomes its UTF-16 escapes (`\u` and four hex digits).
        let mut escaped = String::with_capacity(compact.len());
        for character in compact.chars() {
            if character.is_ascii() {
                escaped.push(character);
            } else {
                let mut units = [0_u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    escaped.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
        Ok(escaped)
    }
}

/// `value` with every object's keys in code-point order.
pub(super) fn sorted(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), sorted(&entries[key])))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}
