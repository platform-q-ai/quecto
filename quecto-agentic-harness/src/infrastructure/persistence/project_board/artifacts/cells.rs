//! The cells a kind's fields and rows are made of, each with the one
//! spelling it is written in and read back from. Slugs, numbers, times,
//! words and `#n` references go in ASCII cells; an [`Identity`] goes in a
//! text cell as ``Name (`email`)``, the email in a code span so a reader
//! shows it as it is, never as a link. An optional ASCII cell holds [`ABSENT`],
//! only where `-` is no value of its own.
use super::CodecError;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::{
    CellKind, Column, Document, check_text_cell, code_cell_holds,
};

/// What an optional ASCII cell holds when it holds nothing.
pub const ABSENT: &str = "-";

/// A cell `field` refused, and why.
pub fn refused(field: &'static str, problem: impl Into<String>) -> CodecError {
    CodecError::Cell {
        field,
        problem: problem.into(),
    }
}

/// A field or table column of a codec's shape.
pub const fn column(name: &'static str, cell: CellKind) -> Column<'static> {
    Column { name, cell }
}

/// A document's field rows, by name: what a codec's `from_document` reads.
pub struct Fields<'a>(&'a Document);

impl<'a> Fields<'a> {
    pub fn of(document: &'a Document) -> Self {
        Self(document)
    }

    /// The cell of the row `name`, if the document has one (a missing
    /// optional row is "not set").
    pub fn get(&self, name: &str) -> Option<&'a str> {
        self.0
            .fields
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, cell)| cell.as_str())
    }

    /// The cell of the row `name`, which every value of the kind has.
    ///
    /// # Errors
    /// The row is missing.
    pub fn required(&self, name: &'static str) -> Result<&'a str, CodecError> {
        self.get(name).ok_or_else(|| refused(name, "missing"))
    }
}

pub fn slug(slug: &Slug) -> String {
    slug.as_str().to_string()
}

/// # Errors
/// A cell that is no slug.
pub fn read_slug(field: &'static str, cell: &str) -> Result<Slug, CodecError> {
    Slug::parse(cell).map_err(|error| refused(field, error.problem))
}

pub fn number(number: u64) -> String {
    number.to_string()
}

/// A number in decimal, without a sign or a leading zero.
///
/// # Errors
/// Any other spelling, or one past `u64`.
pub fn read_number(field: &'static str, cell: &str) -> Result<u64, CodecError> {
    let digits = !cell.is_empty() && cell.bytes().all(|b| b.is_ascii_digit());
    let canonical = digits && (cell == "0" || !cell.starts_with('0'));
    let problem = "a number in decimal, without a sign or a leading zero";
    match canonical {
        true => cell.parse().map_err(|_| refused(field, problem)),
        false => Err(refused(field, problem)),
    }
}

pub fn time(time: &Timestamp) -> String {
    time.as_str().to_string()
}

/// # Errors
/// A cell that is no timestamp.
pub fn read_time(field: &'static str, cell: &str) -> Result<Timestamp, CodecError> {
    Timestamp::parse(cell).map_err(|error| refused(field, error.problem))
}

/// A word of a closed vocabulary, as it spells itself.
pub fn word(word: &str) -> String {
    word.to_string()
}

/// The word of the vocabulary `parse` admits.
///
/// # Errors
/// A spelling the vocabulary does not have.
pub fn read_word<T>(
    field: &'static str,
    cell: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, CodecError> {
    parse(cell).ok_or_else(|| refused(field, "a word of its vocabulary"))
}

/// A pull request or issue: `#n`.
///
/// # Errors
/// 0, which no pull request or issue is (`read_reference` refuses it).
pub fn reference(field: &'static str, number: u64) -> Result<String, CodecError> {
    match number {
        0 => Err(refused(field, "# and a number from 1")),
        number => Ok(format!("#{number}")),
    }
}

/// # Errors
/// Anything but `#` and a number from 1.
pub fn read_reference(field: &'static str, cell: &str) -> Result<u64, CodecError> {
    let problem = "# and a number from 1";
    let number = cell
        .strip_prefix('#')
        .ok_or_else(|| refused(field, problem))?;
    match read_number(field, number)? {
        0 => Err(refused(field, problem)),
        number => Ok(number),
    }
}

/// An optional cell: [`ABSENT`] for none.
///
/// # Errors
/// A cell that holds [`ABSENT`] itself, which would read back as none.
pub fn optional(field: &'static str, cell: Option<String>) -> Result<String, CodecError> {
    match cell {
        None => Ok(ABSENT.to_string()),
        Some(held) if held == ABSENT => Err(refused(field, "a value other than -, which is none")),
        Some(held) => Ok(held),
    }
}

/// The cell an optional cell holds, `None` for [`ABSENT`].
pub fn read_optional(cell: &str) -> Option<&str> {
    match cell {
        ABSENT => None,
        held => Some(held),
    }
}

/// ``Name (`email`)``. The email holds no space, so the last `` (` ``
/// starts it, whatever the name holds; in a code span a reader shows it as
/// it is (a table's `\|` aside), so it may not hold a backtick.
///
/// # Errors
/// An identity that is not a valid one, or an email with a backtick.
pub fn identity(field: &'static str, identity: &Identity) -> Result<String, CodecError> {
    identity
        .validate(field)
        .map_err(|error| refused(field, error.problem))?;
    debug_assert!(
        !identity.email.contains(' '),
        "a valid email holds no space"
    );
    if !code_cell_holds(&identity.email) {
        return Err(refused(
            field,
            "an email a code span holds, without a backtick",
        ));
    }
    let cell = format!("{} (`{}`)", identity.name, identity.email);
    // The inline rules, here, so the error names the field.
    check_text_cell(&cell).map_err(|error| refused(field, error.to_string()))?;
    Ok(cell)
}

/// The identity ``Name (`email`)`` holds: the email what follows the last
/// `` (` ``, up to the final `` `) ``.
///
/// # Errors
/// A cell of another shape, or an identity that is not a valid one.
pub fn read_identity(field: &'static str, cell: &str) -> Result<Identity, CodecError> {
    let shaped = cell
        .strip_suffix("`)")
        .and_then(|cell| cell.rsplit_once(" (`"));
    let (name, email) = shaped.ok_or_else(|| refused(field, "Name (`email`)"))?;
    let identity = Identity {
        name: name.to_string(),
        email: email.to_string(),
    };
    identity
        .validate(field)
        .map_err(|error| refused(field, error.problem))?;
    Ok(identity)
}
