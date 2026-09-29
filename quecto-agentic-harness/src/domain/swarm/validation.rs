//! Input validation the board applies before storing (#2266): the bounded
//! text rule (`swarm_store.bounded`) and the evidence-criteria rule
//! (`Workbench._criteria`).
use serde_json::Value;

use super::records::{Criterion, CriterionKind};
use super::{BoardError, RefusalKind};

/// The most bytes the encoded criteria list may take.
pub const CRITERIA_MAX_BYTES: usize = 16384;
/// The most bytes a criterion id may take.
pub const CRITERION_ID_MAX_BYTES: usize = 128;
/// `bounded`'s default maximum.
pub const TEXT_MAX_BYTES: usize = 8192;
/// The most bytes a member id may take (`Workbench._admit`'s
/// `bounded(member, 'member', 128)`): no longer id is ever a member's.
pub const MEMBER_ID_MAX_BYTES: usize = 128;

/// Python `str.isspace` for one character: the characters `str.strip()`
/// removes. Unlike Rust's `char::is_whitespace` it includes the information
/// separators U+001C..U+001F; neither counts U+200B.
pub fn python_space(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | '\u{1c}'..='\u{1f}' | ' ' | '\u{85}' | '\u{a0}'
            | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
            | '\u{205f}' | '\u{3000}'
    )
}

/// Whether Python's `text.strip()` leaves something.
pub fn has_content(text: &str) -> bool {
    text.chars().any(|character| !python_space(character))
}

/// `swarm_store.bounded`: a JSON string with content, at most `maximum`
/// UTF-8 bytes. Any other JSON value is refused with the same message.
pub fn bounded<'a>(value: &'a Value, label: &str, maximum: usize) -> Result<&'a str, BoardError> {
    match value {
        Value::String(text) => bounded_text(text, label, maximum),
        _ => Err(too_long(label, maximum)),
    }
}

/// [`bounded`] for text already known to be a string.
pub fn bounded_text<'a>(text: &'a str, label: &str, maximum: usize) -> Result<&'a str, BoardError> {
    if has_content(text) && text.len() <= maximum {
        return Ok(text);
    }
    Err(too_long(label, maximum))
}

fn too_long(label: &str, maximum: usize) -> BoardError {
    BoardError::new(
        RefusalKind::Invalid,
        format!("{label} must be nonempty and at most {maximum} bytes"),
    )
}

/// `Workbench._criteria`: a nonempty list of `{id, kind, description}` with
/// distinct ids, checked entry by entry in Python's order, then the encoded
/// list's size. `encoded_len` is the byte length of the list as the board
/// encodes it: Python's `json.dumps(value, sort_keys=True,
/// separators=(',', ':'))` with `ensure_ascii` (so `é` costs 6 bytes as
/// `\u00e9`), which is the board codec's `encode` (#2268), never a
/// `serde_json` encoding. Taking the length keeps this rule codec-free. The parsed
/// criteria are for decisions only: callers persist the ORIGINAL `value`
/// (Python stores the list it was given, extra keys included), never a
/// re-encoding of the returned vector.
pub fn criteria(value: &Value, encoded_len: usize) -> Result<Vec<Criterion>, BoardError> {
    let entries = match value {
        Value::Array(entries) if !entries.is_empty() => entries,
        _ => {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "explicit evidence criteria required",
            ));
        }
    };
    let mut parsed: Vec<Criterion> = Vec::with_capacity(entries.len());
    for entry in entries {
        let criterion = criterion(entry)?;
        if parsed.iter().any(|seen| seen.id == criterion.id) {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "duplicate criterion id",
            ));
        }
        parsed.push(criterion);
    }
    debug_assert!(
        parsed
            .iter()
            .enumerate()
            .all(|(index, criterion)| parsed[..index]
                .iter()
                .all(|earlier| earlier.id != criterion.id)),
        "criterion ids are unique after validation"
    );
    if encoded_len <= CRITERIA_MAX_BYTES {
        return Ok(parsed);
    }
    Err(too_long("criteria", CRITERIA_MAX_BYTES))
}

fn criterion(entry: &Value) -> Result<Criterion, BoardError> {
    let kind = match entry {
        Value::Object(fields) => fields
            .get("kind")
            .and_then(Value::as_str)
            .and_then(CriterionKind::parse),
        _ => None,
    };
    let Some(kind) = kind else {
        return Err(BoardError::new(
            RefusalKind::Invalid,
            "criteria distinguish command checks from parent-reviewed requirements",
        ));
    };
    let id = bounded(&entry["id"], "criterion id", CRITERION_ID_MAX_BYTES)?;
    let description = bounded(
        &entry["description"],
        "criterion description",
        TEXT_MAX_BYTES,
    )?;
    Ok(Criterion {
        id: id.to_owned(),
        kind,
        description: description.to_owned(),
    })
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod validation_tests;
