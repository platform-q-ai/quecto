//! Board artifacts as canonical markdown files (epic #2481, M2b). Each
//! kind's codec maps its domain value to a markdown [`Document`] and back;
//! [`encode`] and [`decode`] put the file around it. A file is read only
//! if it is of the codec's kind and schema, and only if the value it reads
//! as writes the very document it was read from: anything else is refused,
//! never taken as something near it.
pub mod cells;
pub mod change;
pub mod plan;
pub mod review;
pub mod run;

use super::markdown::{Document, MarkdownError, Shape, parse, render, schema_of};

/// Why a value or a file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// The document or file breaks the markdown rules.
    Markdown(MarkdownError),
    /// A cell does not hold what its field does.
    Cell {
        field: &'static str,
        problem: String,
    },
    /// No marker of the codec's kind opens the file, or the codec's own
    /// kind and schema disagree with what it wrote.
    Schema,
    /// A newer quecto wrote it, at a schema past the one this codec reads.
    /// `found` may be past what a `u32` holds.
    NewerSchema { found: u64, known: u32 },
    /// A schema before the one this codec reads, which none reads.
    UnknownSchema { found: u64 },
    /// Read, but not the file its value writes: its document is not the
    /// one the value gives, or its bytes are not the ones it renders to.
    NotCanonical,
}

impl From<MarkdownError> for CodecError {
    fn from(error: MarkdownError) -> Self {
        match error {
            MarkdownError::NotCanonical => Self::NotCanonical,
            other => Self::Markdown(other),
        }
    }
}

/// One kind's codec: its value, its marker's kind, the one schema it reads
/// and writes, its file's shape, and the mapping both ways.
pub trait ArtifactCodec {
    /// Compared with what its file gives back.
    type Value: PartialEq;
    const SCHEMA: u32;
    const SHAPE: Shape<'static>;

    /// The document `value` is, at [`Self::SCHEMA`].
    ///
    /// # Errors
    /// What in `value` no document of the kind holds.
    fn to_document(value: &Self::Value) -> Result<Document, CodecError>;

    /// The value `document` holds.
    ///
    /// # Errors
    /// What in `document` is not a value of the kind.
    fn from_document(document: &Document) -> Result<Self::Value, CodecError>;
}

/// The file `value` is.
///
/// # Errors
/// What in `value` the kind's document or a file refuses.
pub fn encode<C: ArtifactCodec>(value: &C::Value) -> Result<String, CodecError> {
    let document = C::to_document(value)?;
    let file = match document.schema == C::SCHEMA {
        // `render` reads what it writes back, release builds included.
        true => render(&document, &C::SHAPE)?,
        false => return Err(CodecError::Schema),
    };
    // The file reads back as `document`; it is taken only if that document
    // reads as `value` and writes itself again (as `decode` asks), so
    // nothing is stored that `decode` would refuse or read as another
    // value. Both checks: a value's equality may be looser than its
    // document (a repository's ignores case).
    let back = C::from_document(&document)?;
    match back == *value && C::to_document(&back)? == document {
        true => Ok(file),
        false => Err(CodecError::NotCanonical),
    }
}

/// The value the file `bytes` holds: its schema read from its marker alone,
/// then its document, then the value, taken only if it writes the same
/// document again.
///
/// # Errors
/// A file of another kind or schema, one the markdown rules refuse, a
/// document that is not a value of the kind, or `NotCanonical`.
pub fn decode<C: ArtifactCodec>(bytes: &[u8]) -> Result<C::Value, CodecError> {
    match schema_of(bytes, C::SHAPE.kind) {
        Some(found) if found == u64::from(C::SCHEMA) => {}
        Some(found) if found > u64::from(C::SCHEMA) => {
            return Err(CodecError::NewerSchema {
                found,
                known: C::SCHEMA,
            });
        }
        Some(found) => return Err(CodecError::UnknownSchema { found }),
        None => return Err(CodecError::Schema),
    }
    let document = parse(bytes, &C::SHAPE)?;
    debug_assert_eq!(
        document.schema,
        C::SCHEMA,
        "the marker read is the one parsed"
    );
    let value = C::from_document(&document)?;
    match C::to_document(&value)? == document {
        true => Ok(value),
        false => Err(CodecError::NotCanonical),
    }
}

#[cfg(test)]
#[path = "artifacts_tests.rs"]
mod tests;
