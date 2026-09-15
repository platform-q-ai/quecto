//! The file adapter of the sessions export port (#1859, #1974): writes one
//! raw export as a fresh directory under the export root — `records.jsonl`
//! (one format-1 record per line, SHA-256 over the bytes written) and
//! `manifest.json` — on a blocking worker, and returns where it landed.
//! A failed export leaves no directory behind; an export over the size
//! bound is refused. Nothing here selects records or checks the session:
//! that is the use case's.
use crate::application::sessions::dto::{ExportManifest, ExportRecord, RawExportReceipt};
use crate::application::sessions::ports::export::{ExportOutcome, SessionExportPort};
use crate::domain::error::DomainError;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

#[path = "session_export_records.rs"]
mod records;

/// The largest records file an export may write.
const MAX_EXPORT_BYTES: u64 = 256 * 1024 * 1024;

pub struct FileSessionExport {
    root: PathBuf,
}

impl FileSessionExport {
    /// An exporter writing under `root` (created on first export).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

/// A writer task that did not complete (it panicked or was cancelled):
/// reported by the runtime's own text, unprefixed, exactly as the report
/// path always surfaced it.
fn join_failure(error: tokio::task::JoinError) -> DomainError {
    DomainError::Other(error.to_string())
}

impl SessionExportPort for FileSessionExport {
    fn write_export(
        &self,
        records: Vec<ExportRecord>,
        manifest: ExportManifest,
    ) -> ExportOutcome<'_> {
        let root = self.root.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || write(&root, &records, &manifest))
                .await
                .map_err(join_failure)?
        })
    }
}

fn write(
    root: &Path,
    records: &[ExportRecord],
    manifest: &ExportManifest,
) -> Result<RawExportReceipt, DomainError> {
    write_bounded(root, records, manifest, MAX_EXPORT_BYTES)
}

/// `write` with an explicit size bound. Production always uses
/// [`MAX_EXPORT_BYTES`]; the refusal test lowers it rather than building a
/// 256 MiB message.
fn write_bounded(
    root: &Path,
    records: &[ExportRecord],
    manifest: &ExportManifest,
    max_bytes: u64,
) -> Result<RawExportReceipt, DomainError> {
    let io_error = |error: std::io::Error| DomainError::Tool(format!("session export: {error}"));
    std::fs::create_dir_all(root).map_err(io_error)?;
    let root = root.canonicalize().map_err(io_error)?;
    let temporary = tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(root)
        .map_err(io_error)?;
    let path = temporary.path().join("records.jsonl");
    let mut output = std::fs::File::create(&path).map_err(io_error)?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    for record in records {
        let mut line = serde_json::to_vec(&records::record_json(record))
            .map_err(|error| DomainError::Tool(error.to_string()))?;
        line.push(b'\n');
        bytes = bytes.saturating_add(line.len() as u64);
        if bytes <= max_bytes {
            output.write_all(&line).map_err(io_error)?;
            hash.update(&line);
        } else {
            return Err(DomainError::Tool(format!(
                "session export exceeds {} MiB; use paginated recovery",
                max_bytes / (1024 * 1024)
            )));
        }
    }
    output.sync_all().map_err(io_error)?;
    let sha256 = format!("{:x}", hash.finalize());
    let manifest_path = temporary.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&records::manifest_json(
            manifest,
            &sha256,
            bytes,
            &super::runtime_identity::current(),
        ))
        .map_err(|error| DomainError::Tool(error.to_string()))?,
    )
    .map_err(io_error)?;
    let directory = temporary.keep();
    Ok(RawExportReceipt {
        records_path: directory.join("records.jsonl"),
        manifest_path: directory.join("manifest.json"),
        sha256,
        bytes,
    })
}

#[cfg(test)]
#[path = "session_export_tests.rs"]
mod tests;
