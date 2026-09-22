//! The one whole-file read of a session record that discovery makes — by the
//! home catalogue's strict validation and by the store's summary walk alike.
//! An error here is an I/O failure (EMFILE, EACCES, EIO, a delete racing the
//! read…): it says nothing about the record's CONTENT, so neither caller may
//! remember it (R2-H1) — it is reported for that answer and the read is made
//! again on the next query. Only bytes this returned can earn a cached verdict.
use std::path::Path;

pub(in crate::infrastructure::persistence) fn read_record(path: &Path) -> std::io::Result<Vec<u8>> {
    #[cfg(any(test, feature = "test-support"))]
    if faults::take(path) {
        return Err(std::io::Error::other("injected read failure"));
    }
    let bytes = std::fs::read(path);
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
mod faults {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    static ARMED: Mutex<Option<HashMap<PathBuf, usize>>> = Mutex::new(None);
    static REWRITES: Mutex<Option<HashMap<PathBuf, Vec<u8>>>> = Mutex::new(None);
    pub(super) fn arm_rewrite(path: &Path, bytes: Vec<u8>) {
        let mut armed = REWRITES.lock().expect("rewrite table lock");
        armed
            .get_or_insert_with(HashMap::new)
            .insert(path.to_path_buf(), bytes);
    }
    pub(super) fn rewrite_after(path: &Path) {
        let mut armed = REWRITES.lock().expect("rewrite table lock");
        if let Some(bytes) = armed.as_mut().and_then(|table| table.remove(path)) {
            std::fs::write(path, bytes).expect("rewrite under the read");
        }
    }

    pub(super) fn arm(path: &Path, count: usize) {
        let mut armed = ARMED.lock().expect("fault table lock");
        armed
            .get_or_insert_with(HashMap::new)
            .insert(path.to_path_buf(), count);
    }

    pub(super) fn take(path: &Path) -> bool {
        let mut armed = ARMED.lock().expect("fault table lock");
        let Some(left) = armed.as_mut().and_then(|table| table.get_mut(path)) else {
            return false;
        };
        let failing = *left > 0;
        *left = left.saturating_sub(1);
        failing
    }
}
