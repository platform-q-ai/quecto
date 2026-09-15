//! Export a retained session report (#1859): the latest eligible assistant
//! report a supervisor reads, its typed recovery metadata, and the raw
//! export the report may optionally be accompanied by — what the export
//! records, what its manifest states, and the receipt the writer returns.
//!
//! The preview is bounded to [`REPORT_PREVIEW_BYTES`] bytes on a character
//! boundary; a truncated preview carries the recovery ref (`get_message`
//! from `offset`) so the rest is reachable without paging history.
use std::path::PathBuf;

use crate::domain::error::DomainError;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;
use crate::domain::session::SpillEntry;

/// The report preview budget, in bytes.
pub const REPORT_PREVIEW_BYTES: usize = 8192;

/// The report: the latest assistant message that is neither a tool-call
/// step nor blank, resolved to its fullest retained copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPreview {
    pub message_id: MessageId,
    /// The first [`REPORT_PREVIEW_BYTES`] of the content on a character
    /// boundary (the whole content when it fits).
    pub content: String,
    pub content_truncated: bool,
    pub full_length_bytes: usize,
}

impl ReportPreview {
    /// The bounded, UTF-8-safe preview of `message`.
    pub fn of(message: &Message) -> Self {
        let mut end = message.content.len().min(REPORT_PREVIEW_BYTES);
        while !message.content.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            message_id: MessageId::from(message.id().to_string()),
            content: message.content[..end].to_string(),
            content_truncated: end < message.content.len(),
            full_length_bytes: message.content.len(),
        }
    }

    /// Where a truncated preview continues: the message from the preview's
    /// end. `None` when the preview is the whole content.
    pub fn recovery(&self) -> Option<ReportRecovery> {
        self.content_truncated.then(|| ReportRecovery {
            message_id: self.message_id.clone(),
            offset: self.content.len(),
        })
    }
}

/// Typed recovery metadata: the message recovery continues from `offset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRecovery {
    pub message_id: MessageId,
    pub offset: usize,
}

/// One `get_report` answer: the report when the session holds one, and
/// the raw-export receipt when one was requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReport {
    pub report: Option<ReportPreview>,
    pub raw_export: Option<RawExportReceipt>,
}

/// One line of a raw export: a retained message (the ledger's full copy
/// before the live entry) or one spill entry with its content.
#[derive(Debug, Clone)]
pub enum ExportRecord {
    Message(Box<Message>),
    Spill(SpillEntry),
}

/// What the export manifest states about the records: the snapshot they
/// were read at, their counts, and the format and consistency promises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportManifest {
    pub epoch: u64,
    pub revision: u64,
    pub record_count: usize,
    pub spill_count: usize,
}

impl ExportManifest {
    /// The export format this manifest describes.
    pub const FORMAT: u64 = 1;
    /// What an export covers.
    pub const SCOPE: &'static str = "retained live and full-message ledger plus available spill entries; previously evicted or cleared data is not reconstructed";
    /// What an export promises about spill entries.
    pub const SPILL_CONSISTENCY: &'static str =
        "entries read after the message snapshot; concurrent appends may be absent";
}

/// What the writer returns: where the records and manifest landed and the
/// checksum and size of the records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawExportReceipt {
    pub records_path: PathBuf,
    pub manifest_path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

/// Why a report or its raw export was refused.
#[derive(Debug)]
pub enum ReportError {
    /// The loop was composed without an export directory.
    ExportUnavailable,
    /// Both bounded export slots are taken.
    ExportBusy,
    /// The retention store could not be listed or recalled.
    Spill(DomainError),
    /// A listed spill entry was gone by the time it was recalled.
    SpillDisappeared(String),
    /// The session was replaced while the export was being assembled.
    EpochChanged,
    /// The writer failed.
    Writer(DomainError),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExportUnavailable => f.write_str("session export directory unavailable"),
            Self::ExportBusy => {
                f.write_str("two raw exports are already running; retry after completion")
            }
            Self::Spill(error) | Self::Writer(error) => write!(f, "{error}"),
            Self::SpillDisappeared(id) => write!(f, "spill disappeared during export: {id}"),
            Self::EpochChanged => {
                f.write_str("session changed during export; retry against the new epoch")
            }
        }
    }
}

#[cfg(test)]
#[path = "session_report_tests.rs"]
mod tests;
