//! Test seams of the one record read (#2042): the next `count` reads of a
//! path fail as a transient I/O failure would, or the file is rewritten
//! right after its next read, as a save racing the read would.
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
