//! The one configuration writer (#2024): a JSON document laid down
//! minimal-diff (the file's own indentation kept, two spaces for a new
//! file, key order and unknown keys as the document carries them, a
//! trailing newline) and atomically (tmp + fsync + rename through
//! `atomic_write`). It serves the configuration capability's
//! [`ConfigDocumentWriter`] port and the tool-policy persistence hook
//! (`tool_policy`), which patches only `tools.policy.entries`.

use std::path::Path;

use crate::application::configuration::ports::ConfigDocumentWriter;
use crate::infrastructure::atomic_write::atomic_write;

pub mod tool_policy;

/// The mode a configuration file is created with: it may hold API keys.
const NEW_CONFIG_MODE: u32 = 0o600;

#[derive(Debug, Default, Clone, Copy)]
pub struct JsonDocumentWriter;

impl ConfigDocumentWriter for JsonDocumentWriter {
    fn write(&self, path: &Path, document: &serde_json::Value) -> Result<Vec<u8>, String> {
        write_document(path, document)
    }
}

/// Render `document` in the layout `path` already uses and replace the
/// file atomically, returning the bytes written. An existing file keeps
/// its mode; a new one is private to the user.
pub fn write_document(path: &Path, document: &serde_json::Value) -> Result<Vec<u8>, String> {
    // A symlinked config (dotfiles) is written through the link: the rename
    // would otherwise replace the link with a plain file and leave the
    // linked copy stale.
    let target = resolve_symlink(path)?;
    let path = target.as_path();
    let existing = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let bytes = render(document, existing.as_deref());
    let mode = existing_mode(path).unwrap_or(NEW_CONFIG_MODE);
    atomic_write(path, &bytes, Some(mode)).map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// The file a write lands in: the final target when `path` is a symlink
/// (a dangling link is an error naming it), else `path` itself.
fn resolve_symlink(path: &Path) -> Result<std::path::PathBuf, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::canonicalize(path).map_err(|error| {
                format!(
                    "{} is a symlink that cannot be resolved: {error}",
                    path.display()
                )
            })
        }
        _ => Ok(path.to_path_buf()),
    }
}

/// Pretty-print with the indentation the existing file used (two spaces
/// for a new or compact file) and a trailing newline, so a file already in
/// that shape changes only on the lines whose values changed.
fn render(document: &serde_json::Value, existing: Option<&[u8]>) -> Vec<u8> {
    let indent = existing
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(detect_indent)
        .unwrap_or_else(|| "  ".to_string());
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
    serde::Serialize::serialize(document, &mut serializer).expect("a JSON value serializes");
    bytes.push(b'\n');
    bytes
}

/// The leading whitespace of the first indented line, when the text has
/// one; a compact single-line document yields `None`.
fn detect_indent(text: &str) -> Option<String> {
    text.lines()
        .skip(1)
        .map(|line| &line[..line.len() - line.trim_start().len()])
        .find(|indent| !indent.is_empty())
        .map(str::to_string)
}

#[cfg(unix)]
fn existing_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn existing_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
