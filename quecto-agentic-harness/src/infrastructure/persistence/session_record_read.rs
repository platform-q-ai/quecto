//! The one whole-file read of a session record that discovery makes — by the
//! home catalogue's strict validation and by the store's summary walk alike.
//! An error here is an I/O failure (EMFILE, EACCES, EIO, a delete racing the
//! read…): it says nothing about the record's CONTENT, so neither caller may
//! remember it (R2-H1) — it is reported for that answer and the read is made
//! again on the next query. Only bytes this returned can earn a cached verdict.
use std::path::Path;

/// Records above this are never read (#2042): a sparse or runaway file must
/// not be pulled into memory once per process by each half. A verdict on the
/// STAMP — the size is in it — so it is remembered like any content verdict.
pub const MAX_RECORD_BYTES: u64 = 64 * 1024 * 1024;

/// Why a record was not read: an I/O failure (no verdict, retried next
/// time) or a size above the cap (a verdict on this version).
#[derive(Debug)]
pub enum ReadRefusal {
    Io(std::io::Error),
    TooLarge { len: u64, cap: u64 },
}

impl std::fmt::Display for ReadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::TooLarge { len, cap } => write!(f, "record too large: {len} bytes, cap {cap}"),
        }
    }
}

/// The one read; `stamp` is the record's, its third field the size.
pub(in crate::infrastructure::persistence) fn read_record(
    path: &Path,
    stamp: &[u64],
) -> Result<Vec<u8>, ReadRefusal> {
    if let Some(&len) = stamp.get(2).filter(|len| **len > MAX_RECORD_BYTES) {
        return Err(ReadRefusal::TooLarge {
            len,
            cap: MAX_RECORD_BYTES,
        });
    }
    #[cfg(any(test, feature = "test-support"))]
    if faults::take(path) {
        return Err(ReadRefusal::Io(std::io::Error::other(
            "injected read failure",
        )));
    }
    let bytes = std::fs::read(path).map_err(ReadRefusal::Io);
    #[cfg(any(test, feature = "test-support"))]
    faults::rewrite_after(path);
    bytes
}

/// Test seam: right after the next read of exactly `path`, the file is
/// rewritten with `bytes` — a save racing the read, which must leave both
/// halves without a remembered verdict (#2042).
#[cfg(any(test, feature = "test-support"))]
pub fn rewrite_after_next_read(path: &Path, bytes: Vec<u8>) {
    faults::arm_rewrite(path, bytes);
}

/// Test seam: the next `count` reads of exactly `path` fail, as a transient
/// I/O failure would — with the file, and therefore its stamp, untouched.
#[cfg(any(test, feature = "test-support"))]
pub fn fail_next_reads(path: &Path, count: usize) {
    faults::arm(path, count);
}

#[cfg(any(test, feature = "test-support"))]
#[path = "session_record_read_faults.rs"]
mod faults;
