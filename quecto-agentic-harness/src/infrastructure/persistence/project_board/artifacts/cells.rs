//! The cells a kind's fields and rows are made of, each with the one
//! spelling it is written in and read back from. Slugs, numbers, times,
//! words and `#n` references go in ASCII cells; an [`Identity`] goes in a
//! text cell as `Name (email)`. An optional ASCII cell holds [`ABSENT`],
//! only where `-` is no value of its own.
use super::CodecError;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// What an optional ASCII cell holds when it holds nothing.
pub const ABSENT: &str = "-";

fn refused(field: &'static str, problem: impl Into<String>) -> CodecError {
    CodecError::Cell {
        field,
        problem: problem.into(),
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
pub fn reference(number: u64) -> String {
    format!("#{number}")
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
pub fn optional(cell: Option<String>) -> String {
    cell.unwrap_or_else(|| ABSENT.to_string())
}

/// The cell an optional cell holds, `None` for [`ABSENT`].
pub fn read_optional(cell: &str) -> Option<&str> {
    match cell {
        ABSENT => None,
        held => Some(held),
    }
}

/// `Name (email)`. The email holds no space, so the last ` (` starts it,
/// whatever parentheses the name holds.
///
/// # Errors
/// An identity that is not a valid one.
pub fn identity(field: &'static str, identity: &Identity) -> Result<String, CodecError> {
    identity
        .validate(field)
        .map_err(|error| refused(field, error.problem))?;
    debug_assert!(
        !identity.email.contains(' '),
        "a valid email holds no space"
    );
    Ok(format!("{} ({})", identity.name, identity.email))
}

/// The identity `Name (email)` holds: the email what follows the last
/// ` (`, up to the final `)`.
///
/// # Errors
/// A cell of another shape, or an identity that is not a valid one.
pub fn read_identity(field: &'static str, cell: &str) -> Result<Identity, CodecError> {
    let shaped = cell
        .strip_suffix(')')
        .and_then(|cell| cell.rsplit_once(" ("));
    let (name, email) = shaped.ok_or_else(|| refused(field, "Name (email)"))?;
    let identity = Identity {
        name: name.to_string(),
        email: email.to_string(),
    };
    identity
        .validate(field)
        .map_err(|error| refused(field, error.problem))?;
    Ok(identity)
}
