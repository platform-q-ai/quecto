//! A moment on the board: RFC 3339 UTC to the second, `2026-10-10T09:30:00Z`.
use super::schema_error::SchemaError;
use serde::{Deserialize, Serialize};

/// One fixed spelling, so text order is time order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Timestamp(String);

const SHAPE: &[u8; 20] = b"dddd-dd-ddTdd:dd:ddZ";

impl Timestamp {
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        let shaped = text.len() == SHAPE.len()
            && text.bytes().zip(SHAPE).all(|(byte, want)| match want {
                b'd' => byte.is_ascii_digit(),
                literal => byte == *literal,
            });
        // The shape is ASCII, so the calendar check sees exactly these bytes.
        if shaped && humantime::parse_rfc3339(text).is_ok() {
            debug_assert!(text.is_ascii() && text.ends_with('Z'));
            Ok(Self(text.to_string()))
        } else {
            Err(SchemaError::new(
                "timestamp",
                format!("{text:?} must be YYYY-MM-DDTHH:MM:SSZ"),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Timestamp {
    type Error = SchemaError;
    fn try_from(text: String) -> Result<Self, SchemaError> {
        Self::parse(&text)
    }
}

impl From<Timestamp> for String {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.0
    }
}

#[cfg(test)]
#[path = "timestamp_tests.rs"]
mod tests;
