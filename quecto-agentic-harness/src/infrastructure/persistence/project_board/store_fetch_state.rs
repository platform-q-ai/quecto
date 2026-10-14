//! What a partial clone remembers between reads of the board (#2481 S10b
//! review round 1), in `.git/quecto/project-board/fetch-<remote>`: the last
//! board commit it read whole, from which a new head is fetched as what
//! changed, and the blobs a filtered fetch left on the remote, so no read
//! fetches a folder holding one whole again.
//!
//! It is a cache, never trusted further than a fetch would be: a commit it
//! names that this clone does not hold is not used, a file that is not
//! exactly this shape is no file, and every failure to keep it only makes
//! the next read fetch more. A blob it names wrongly is at worst set aside
//! and carried by its id, never dropped.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{
    FILTER_BYTES, MAX_BOARD_ENTRIES,
};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

/// The file's first line, before the filter limit its blobs were left
/// under: a state of another limit is no state.
const HEADER: &str = "quecto-board-fetch 1";
/// The most blobs a state names: one for each entry a read takes in
/// `tasks/` and in all the folders.
const MAX_STATE_ENTRIES: usize = MAX_READ_ENTRIES + MAX_BOARD_ENTRIES;
/// The most bytes of a state: its header, a commit and the most blobs,
/// each of the longest ids (SHA-256's 64 digits).
const MAX_STATE_BYTES: usize = HEADER.len()
    + 1
    + digits(FILTER_BYTES)
    + 1
    + "read \n".len()
    + 64
    + MAX_STATE_ENTRIES * ("omitted \n".len() + 64);

/// The decimal digits of `n`.
const fn digits(mut n: usize) -> usize {
    let mut count = 1;
    while n >= 10 {
        n /= 10;
        count += 1;
    }
    count
}

/// The state `bytes` hold, if they are within [`MAX_STATE_BYTES`] and
/// exactly a state's shape.
fn read_bounded(bytes: &[u8]) -> Option<FetchState> {
    match bytes.len() <= MAX_STATE_BYTES {
        true => FetchState::parse(std::str::from_utf8(bytes).ok()?),
        false => None,
    }
}

/// This thread's number, as its id spells it.
fn thread_number() -> String {
    let id = format!("{:?}", std::thread::current().id());
    id.chars().filter(char::is_ascii_digit).collect()
}

/// Names each draft of this process apart, across threads and stores.
static DRAFTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What a partial clone remembers between reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct FetchState {
    /// The last board commit read whole.
    pub read: Option<String>,
    /// Blobs of that board a filtered fetch left on the remote.
    pub omitted: BTreeSet<String>,
}

impl FetchState {
    /// The state `text` spells, if it is exactly this shape: the header,
    /// at most one `read <commit>`, then `omitted <blob>` lines, each an
    /// object id, no more than a board holds entries.
    pub(super) fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        (lines.next()? == format!("{HEADER} {FILTER_BYTES}")).then_some(())?;
        let mut state = Self::default();
        for line in lines {
            match line.split_once(' ') {
                Some(("read", oid)) if state.read.is_none() && state.omitted.is_empty() => {
                    state.read = Some(object_id(oid).ok()?);
                }
                Some(("omitted", oid)) if state.omitted.len() < MAX_STATE_ENTRIES => {
                    state.omitted.insert(object_id(oid).ok()?);
                }
                _ => return None,
            }
        }
        Some(state)
    }

    pub(super) fn render(&self) -> String {
        let read = self.read.iter().map(|oid| format!("read {oid}\n"));
        let omitted = self.omitted.iter().map(|oid| format!("omitted {oid}\n"));
        std::iter::once(format!("{HEADER} {FILTER_BYTES}\n"))
            .chain(read)
            .chain(omitted)
            .collect()
    }
}

impl GitBoardStore {
    /// Where this remote's state lies.
    fn state_path(&self) -> Option<std::path::PathBuf> {
        // A remote's name is letters, digits and `._-`, starting with a
        // letter or digit (`GitBoardStore::new`): one file name.
        let dir = self.common_dir().ok()?.join("quecto").join("project-board");
        Some(dir.join(format!("fetch-{}", self.remote)))
    }

    /// The state a previous read kept, or none.
    pub(super) fn fetch_state(&self) -> FetchState {
        let read = || -> Option<FetchState> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(self.state_path()?)
                .ok()?;
            let mut bytes = vec![];
            let cap = u64::try_from(MAX_STATE_BYTES + 1).ok()?;
            file.take(cap).read_to_end(&mut bytes).ok()?;
            read_bounded(&bytes)
        };
        read().unwrap_or_default()
    }

    /// Keeps `state` for the next read, when it differs from `before`; a
    /// failure only makes the next read fetch more.
    pub(super) fn keep_fetch_state(&self, before: &FetchState, state: &FetchState) {
        if before == state {
            return;
        }
        let Some(path) = self.state_path() else {
            return;
        };
        let Some(dir) = path.parent() else {
            return;
        };
        if std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .is_err()
        {
            return;
        }
        // A draft of its own: this process, this thread, this keeping.
        let draft = dir.join(format!(
            "fetch-{}.{}.{}.{}.draft",
            self.remote,
            std::process::id(),
            thread_number(),
            DRAFTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&draft)
            .and_then(|mut file| file.write_all(state.render().as_bytes()))
            .and_then(|()| std::fs::rename(&draft, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&draft);
        }
    }
}

#[cfg(test)]
#[path = "store_fetch_state_tests.rs"]
mod tests;
