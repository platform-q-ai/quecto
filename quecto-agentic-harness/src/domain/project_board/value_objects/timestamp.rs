//! A moment on the board: RFC 3339 UTC to the second, `2026-10-10T09:30:00Z`.
use super::schema_error::SchemaError;
use std::time::Duration;

/// One fixed spelling, so text order is time order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(String);

const SHAPE: &[u8; 20] = b"dddd-dd-ddTdd:dd:ddZ";
/// The last moment the fixed spelling can hold.
const LAST: &str = "9999-12-31T23:59:59Z";

impl Timestamp {
    /// From 1970 to 9999, seconds 00 to 59 (no leap second).
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        let shaped = text.len() == SHAPE.len()
            && text.bytes().zip(SHAPE).all(|(byte, want)| match want {
                b'd' => byte.is_ascii_digit(),
                literal => byte == *literal,
            });
        let second_in_minute = shaped && matches!(text.as_bytes()[17], b'0'..=b'5');
        // The shape is ASCII, so the calendar check sees exactly these bytes.
        if second_in_minute && humantime::parse_rfc3339(text).is_ok() {
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

    /// The seconds from `earlier` to this moment: below zero when this one
    /// comes first (epic #2481 T4, a claim's age and time left). Every
    /// spellable span fits an `i64` many times over.
    pub fn seconds_since(&self, earlier: &Timestamp) -> i64 {
        let moment = |time: &Timestamp| {
            humantime::parse_rfc3339(&time.0).expect("a parsed timestamp reads back")
        };
        let signed = |span: Duration| i64::try_from(span.as_secs()).unwrap_or(i64::MAX);
        match moment(self).duration_since(moment(earlier)) {
            Ok(after) => signed(after),
            Err(before) => -signed(before.duration()),
        }
    }

    /// This moment `seconds` earlier, refused before 1970 (#2705 B9).
    pub fn minus_seconds(&self, seconds: u64) -> Result<Self, SchemaError> {
        let start = humantime::parse_rfc3339(&self.0).expect("a parsed timestamp reads back");
        match start.checked_sub(Duration::from_secs(seconds)) {
            Some(earlier) if earlier >= std::time::UNIX_EPOCH => {
                Self::parse(&humantime::format_rfc3339_seconds(earlier).to_string())
            }
            _ => Err(SchemaError::new("timestamp", "must not be before 1970")),
        }
    }

    /// This moment `seconds` later, refused past the year 9999.
    pub fn plus_seconds(&self, seconds: u64) -> Result<Self, SchemaError> {
        let start = humantime::parse_rfc3339(&self.0).expect("a parsed timestamp reads back");
        let last = humantime::parse_rfc3339(LAST).expect("the last spellable moment parses");
        // Formatting past the year 9999 panics: only a moment up to it is spelt.
        match start.checked_add(Duration::from_secs(seconds)) {
            Some(later) if later <= last => {
                Self::parse(&humantime::format_rfc3339_seconds(later).to_string())
            }
            _ => Err(SchemaError::new(
                "timestamp",
                format!("must not pass {LAST}"),
            )),
        }
    }
}

#[cfg(test)]
#[path = "timestamp_tests.rs"]
mod tests;
