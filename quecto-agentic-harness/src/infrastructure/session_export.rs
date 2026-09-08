//! Writes explicit diagnostic exports without putting raw transcripts on the wire.
use crate::domain::error::DomainError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

pub fn write(root: &Path, records: Vec<Value>, mut metadata: Value) -> Result<Value, DomainError> {
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
        let mut line =
            serde_json::to_vec(&record).map_err(|error| DomainError::Tool(error.to_string()))?;
        line.push(b'\n');
        bytes = bytes.saturating_add(line.len() as u64);
        if bytes <= 256 * 1024 * 1024 {
            output.write_all(&line).map_err(io_error)?;
            hash.update(&line);
        } else {
            return Err(DomainError::Tool(
                "session export exceeds 256 MiB; use paginated recovery".into(),
            ));
        }
    }
    output.sync_all().map_err(io_error)?;
    metadata["sha256"] = json!(format!("{:x}", hash.finalize()));
    metadata["bytes"] = json!(bytes);
    metadata["runtime"] = json!(super::runtime_identity::current());
    let manifest = temporary.path().join("manifest.json");
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&metadata)
            .map_err(|error| DomainError::Tool(error.to_string()))?,
    )
    .map_err(io_error)?;
    let directory = temporary.keep();
    Ok(
        json!({"path":directory.join("records.jsonl"),"manifest":directory.join("manifest.json"),
        "sha256":metadata["sha256"],"bytes":bytes,"scope":"retained_snapshot"}),
    )
}
