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
    std::fs::read(path)
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
