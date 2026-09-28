//! What an ended child left, as its parent reads it (#2192): the crash
//! record beside its event log and the transcript in the session store,
//! both in the base directory the parent shares with its children (a local
//! child's; a container child's when its store is shared with the parent).
//! Both files are where the child could write, so neither is trusted: a
//! record is read only from a regular file of bounded size, a transcript
//! the same way — its newest part when it is larger than the bound
//! ([`FileSessionStore::load_untrusted`]) — and no link is followed.
//!
//! The transcripts last read are kept and shared — a few children's, the
//! least recently read given up first, together never more than one
//! transcript's bound (#2192 review: readers of two children no longer
//! evict each other) — each keyed on the file's device, inode, length and
//! modification time: paging an unchanged transcript reads nothing again,
//! and one that changed is read afresh. A read that ended on a torn record
//! (the file was being written) is not kept.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::application::subagents::ports::{EndedChildRecords, EndedTranscript, PortFuture};
use crate::domain::crash_record::CrashRecord;
use crate::domain::error::DomainError;
use crate::domain::session_identity::SessionIdentity;

use super::crash_record::{RecordDir, SessionRecords};
use super::session_store::FileSessionStore;
use super::session_store::session_store_records::session_store_bounded::{
    FileStamp, UntrustedRead,
};

/// The most of a transcript an ended child's parent reads.
pub const MAX_TRANSCRIPT_BYTES: u64 = 32 * 1024 * 1024;

/// A transcript read, and the file state it was read from.
type Kept = (SessionIdentity, FileStamp, EndedTranscript);

/// The most transcripts kept at once.
pub const MAX_KEPT_TRANSCRIPTS: usize = 4;

pub struct FileEndedChildRecords {
    base_dir: PathBuf,
    transcripts: Arc<FileSessionStore>,
    max_bytes: u64,
    /// Most recently read last; together at most `max_bytes` of files.
    last: Mutex<std::collections::VecDeque<Kept>>,
}

impl FileEndedChildRecords {
    /// Records under `base_dir`, whose sessions `transcripts` stores.
    pub fn new(base_dir: impl Into<PathBuf>, transcripts: Arc<FileSessionStore>) -> Self {
        Self::with_bound(base_dir, transcripts, MAX_TRANSCRIPT_BYTES)
    }

    /// As [`Self::new`], reading at most `max_bytes` of a transcript.
    pub fn with_bound(
        base_dir: impl Into<PathBuf>,
        transcripts: Arc<FileSessionStore>,
        max_bytes: u64,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            transcripts,
            max_bytes,
            last: Mutex::new(std::collections::VecDeque::new()),
        }
    }

    fn kept(&self, child: &SessionIdentity) -> Option<(FileStamp, EndedTranscript)> {
        let last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        last.iter()
            .find(|(identity, _, _)| identity == child)
            .map(|(_, stamp, transcript)| (*stamp, transcript.clone()))
    }

    /// Keep `child`'s transcript (none: forget it), as the most recently
    /// read, giving up the least recently read ones until at most
    /// [`MAX_KEPT_TRANSCRIPTS`] of at most `max_bytes` together are kept.
    fn keep(&self, child: &SessionIdentity, kept: Option<(FileStamp, EndedTranscript)>) {
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        last.retain(|(identity, _, _)| identity != child);
        if let Some((stamp, transcript)) = kept {
            last.push_back((child.clone(), stamp, transcript));
        }
        let held = |last: &std::collections::VecDeque<Kept>| {
            last.iter().map(|(_, stamp, _)| stamp.length()).sum::<u64>()
        };
        while last.len() > MAX_KEPT_TRANSCRIPTS || (last.len() > 1 && held(&last) > self.max_bytes)
        {
            last.pop_front();
        }
        assert!(last.len() <= MAX_KEPT_TRANSCRIPTS, "the cache is bounded");
    }

    /// The children whose transcripts are kept, least recently read first.
    #[cfg(test)]
    fn kept_children(&self) -> Vec<SessionIdentity> {
        let last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        last.iter()
            .map(|(identity, _, _)| identity.clone())
            .collect()
    }
}

impl std::fmt::Debug for FileEndedChildRecords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileEndedChildRecords")
            .field("base_dir", &self.base_dir)
            .finish_non_exhaustive()
    }
}

impl EndedChildRecords for FileEndedChildRecords {
    fn crash<'a>(
        &'a self,
        child: &'a SessionIdentity,
        writer: Option<u32>,
    ) -> PortFuture<'a, Option<CrashRecord>> {
        let base_dir = self.base_dir.clone();
        let records = SessionRecords::new(child.runtime_key());
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                records.read(&RecordDir::existing(&base_dir).ok()?, writer)
            })
            .await
            .ok()
            .flatten()
        })
    }

    fn has_transcript<'a>(&'a self, child: &'a SessionIdentity) -> PortFuture<'a, bool> {
        Box::pin(self.transcripts.holds_regular_session(child))
    }

    fn transcript<'a>(
        &'a self,
        child: &'a SessionIdentity,
    ) -> PortFuture<'a, Result<Option<EndedTranscript>, DomainError>> {
        Box::pin(async move {
            let kept = self.kept(child);
            let read = self
                .transcripts
                .load_untrusted(
                    child,
                    self.max_bytes,
                    kept.as_ref().map(|(stamp, _)| *stamp),
                )
                .await?;
            match (read, kept) {
                (UntrustedRead::Missing, _) => Ok(None),
                (UntrustedRead::Unchanged, Some((stamp, transcript))) => {
                    // Read again: the most recently read now.
                    self.keep(child, Some((stamp, transcript.clone())));
                    Ok(Some(transcript))
                }
                (UntrustedRead::Unchanged, None) => Err(DomainError::Session(
                    "the transcript was reported unchanged with nothing kept".to_string(),
                )),
                (
                    UntrustedRead::Read {
                        messages,
                        stamp,
                        complete,
                        older_omitted,
                    },
                    _,
                ) => {
                    let transcript = EndedTranscript {
                        messages: Arc::new(messages),
                        older_omitted,
                    };
                    self.keep(child, complete.then(|| (stamp, transcript.clone())));
                    Ok(Some(transcript))
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "ended_child_records_tests.rs"]
mod tests;
