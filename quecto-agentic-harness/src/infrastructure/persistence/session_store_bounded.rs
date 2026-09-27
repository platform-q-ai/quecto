//! A session read on behalf of someone other than its owner (#2192): a
//! parent reading the transcript an ended child left. The file lives where
//! the child could write, so the read trusts nothing about it: it follows no
//! link, never blocks on a FIFO, reads only a regular file, and reads at
//! most the reader's bound — the newest part of a larger file, its older
//! messages reported omitted. A reader that already holds the file as it
//! is (same device, inode, length, modification and status-change time) is
//! told so and nothing is read again.
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;

use super::super::FileSessionStore;

/// What identifies one state of a session file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    device: u64,
    inode: u64,
    length: u64,
    modified_ns: i128,
    /// The status-change time: a rewrite in place moves it even when the
    /// length is kept and the modification time is put back.
    changed_ns: i128,
}

impl FileStamp {
    /// The file's length when it was stamped.
    pub fn length(&self) -> u64 {
        self.length
    }

    fn of(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified_ns: i128::from(metadata.mtime()) * 1_000_000_000
                + i128::from(metadata.mtime_nsec()),
            changed_ns: i128::from(metadata.ctime()) * 1_000_000_000
                + i128::from(metadata.ctime_nsec()),
        }
    }
}

/// The outcome of an untrusted read.
#[derive(Debug)]
pub enum UntrustedRead {
    /// No session is stored under the identity.
    Missing,
    /// The file is as the reader's stamp says: nothing was read.
    Unchanged,
    Read {
        messages: Vec<Message>,
        stamp: FileStamp,
        /// The file ended on a whole record: nothing was being written.
        complete: bool,
        /// Only the newest part of the file was read.
        older_omitted: bool,
    },
}

impl FileSessionStore {
    /// Whether a session for `identity` is stored as a non-empty regular
    /// file — asked of the entry itself, no link followed, nothing read
    /// (#2192 review: an exit note only asks whether a transcript is there).
    pub async fn holds_regular_session(&self, identity: &SessionIdentity) -> bool {
        tokio::fs::symlink_metadata(self.session_path(identity))
            .await
            .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
    }

    /// The session stored for `identity`, read as described above; at most
    /// `max_bytes` of it.
    pub async fn load_untrusted(
        &self,
        identity: &SessionIdentity,
        max_bytes: u64,
        known: Option<FileStamp>,
    ) -> Result<UntrustedRead, DomainError> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        assert!(max_bytes > 0, "a read holds something");
        let failed =
            |e: std::io::Error| DomainError::Session(format!("failed to read session: {e}"));
        let path = self.session_path(identity);
        let opened = tokio::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&path)
            .await;
        let mut file = match opened {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(UntrustedRead::Missing);
            }
            Err(error) => {
                return Err(DomainError::Session(format!(
                    "failed to open session (a link is never followed): {error}"
                )));
            }
        };
        let metadata = file.metadata().await.map_err(failed)?;
        match metadata.file_type().is_file() {
            true => {}
            false => {
                return Err(DomainError::Session(
                    "the session is not a regular file".to_string(),
                ));
            }
        }
        let stamp = FileStamp::of(&metadata);
        if known == Some(stamp) {
            return Ok(UntrustedRead::Unchanged);
        }
        let older_omitted = stamp.length > max_bytes;
        if older_omitted {
            file.seek(std::io::SeekFrom::Start(stamp.length - max_bytes))
                .await
                .map_err(failed)?;
        }
        let mut bytes = Vec::new();
        file.take(max_bytes)
            .read_to_end(&mut bytes)
            .await
            .map_err(failed)?;
        let data = String::from_utf8_lossy(&bytes);
        let complete = match (data.ends_with('\n'), older_omitted) {
            (true, _) => true,
            (false, false) => is_whole_json(&data),
            (false, true) => false,
        };
        let messages = match older_omitted {
            // The first line was cut: skip it, and read the records after.
            true => tail_messages(data.split_once('\n').map_or("", |(_, rest)| rest)),
            false => {
                super::super::parse_session_data(&data)
                    .map_err(|e| DomainError::Session(format!("failed to parse session: {e}")))?
                    .messages
            }
        };
        Ok(UntrustedRead::Read {
            messages,
            stamp,
            complete,
            older_omitted,
        })
    }
}

/// A file that is one JSON document (the legacy snapshot) is whole.
fn is_whole_json(data: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(data).is_ok()
}

/// The messages of the whole records in `data`, the newest part of a JSONL
/// session: a snapshot starts them over, an append adds to them. A torn
/// record ends the read, and so does an append out of place — as the full
/// parse holds a file to its order: an indexed append must start where the
/// messages read so far end. The tail may begin after the snapshot, so
/// until one is read the first indexed append sets where they are.
fn tail_messages(data: &str) -> Vec<Message> {
    use super::SessionRecord;
    let mut messages: Vec<Message> = Vec::new();
    // Where `messages[0]` is in the session, once known.
    let mut first_index: Option<usize> = None;
    for line in data
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
    {
        match serde_json::from_str::<SessionRecord>(line) {
            Ok(SessionRecord::Snapshot(file)) => {
                messages = super::super::session_from_file(file).messages;
                first_index = Some(0);
            }
            Ok(SessionRecord::Append {
                start_index,
                messages: added,
                ..
            }) => {
                let in_place = match (start_index, first_index) {
                    (None, _) => true,
                    (Some(start), Some(first)) => Some(start) == first.checked_add(messages.len()),
                    (Some(start), None) => match start.checked_sub(messages.len()) {
                        Some(first) => {
                            first_index = Some(first);
                            true
                        }
                        None => false,
                    },
                };
                match in_place {
                    true => {
                        messages.extend(added.into_iter().map(super::super::record_to_message));
                    }
                    false => {
                        tracing::warn!(?start_index, "an out-of-order append ends the tail read");
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    messages
}

#[cfg(test)]
#[path = "session_store_bounded_tests.rs"]
mod tests;
