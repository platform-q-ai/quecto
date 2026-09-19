//! A second reading of a line the command parser refused (R2-H3). `1e400` is
//! grammatical JSON that no `f64` holds, and `serde_json` refuses it while
//! lexing — before any field is decoded — so a `search_session_metadata`
//! carrying one in `generation` or `limit` was an uncorrelated `parse_error`
//! its client awaited for ever. Here the line is read once more as a search
//! whose two numbers are raw text; one no `f64` holds becomes the largest or
//! smallest `f64`, which the edge clamps (`limit`) or refuses under the id
//! (`generation`). A search's OWN fields must still decode or the first error
//! stands; an unknown field is skipped unbuilt, whatever it holds (R3-H3).
use super::{AgentCommand, SessionListScopeCommand};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};

#[derive(Deserialize)]
struct RawSearch<'a> {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    query: String,
    #[serde(default)]
    scope: SessionListScopeCommand,
    #[serde(borrow, default)]
    generation: Option<&'a RawValue>,
    #[serde(borrow, default)]
    limit: Option<&'a RawValue>,
}

pub(super) fn rescued(line: &str) -> Option<AgentCommand> {
    let raw: RawSearch<'_> = serde_json::from_str(line).ok()?;
    if raw.kind != "search_session_metadata" {
        return None;
    }
    // Decoded as every search is, from the same fields with usable numbers.
    let usable = serde_json::json!({
        "type": raw.kind, "id": raw.id, "query": raw.query, "scope": raw.scope,
        "generation": value_of(raw.generation), "limit": value_of(raw.limit),
    });
    serde_json::from_value(usable).ok()
}

fn value_of(raw: Option<&RawValue>) -> Value {
    let Some(text) = raw.map(RawValue::get) else {
        return Value::Null;
    };
    serde_json::from_str(text).unwrap_or_else(|_| match text.as_bytes()[0] {
        b'-' => Value::from(f64::MIN),
        b'0'..=b'9' => Value::from(f64::MAX),
        // A composite holding such a number is no number: refused as it was.
        _ => Value::String(text.to_string()),
    })
}
