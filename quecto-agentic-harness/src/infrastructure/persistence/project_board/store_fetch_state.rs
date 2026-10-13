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
use crate::infrastructure::persistence::project_board::board_tree::MAX_BOARD_ENTRIES;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

/// The file's first line.
const HEADER: &str = "quecto-board-fetch 1";
/// The most of the file read: its header, a commit, and one line for each
/// entry a board may hold.
const MAX_STATE_BYTES: usize = 128 + 80 * MAX_BOARD_ENTRIES;

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
        (lines.next()? == HEADER).then_some(())?;
        let mut state = Self::default();
        for line in lines {
            match line.split_once(' ') {
                Some(("read", oid)) if state.read.is_none() && state.omitted.is_empty() => {
                    state.read = Some(object_id(oid).ok()?);
                }
                Some(("omitted", oid)) if state.omitted.len() < MAX_BOARD_ENTRIES => {
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
        std::iter::once(format!("{HEADER}\n"))
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
            let mut text = String::new();
            let cap = u64::try_from(MAX_STATE_BYTES + 1).ok()?;
            file.take(cap).read_to_string(&mut text).ok()?;
            (text.len() <= MAX_STATE_BYTES).then_some(())?;
            FetchState::parse(&text)
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
        let draft = dir.join(format!(
            "fetch-{}.{}.draft",
            self.remote,
            std::process::id()
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
mod tests {
    use super::*;

    #[test]
    fn only_a_state_of_exactly_its_shape_is_read() {
        let (commit, blob) = ("c".repeat(40), "b".repeat(40));
        let state = FetchState {
            read: Some(commit.clone()),
            omitted: BTreeSet::from([blob.clone()]),
        };
        assert_eq!(FetchState::parse(&state.render()), Some(state));
        assert_eq!(
            FetchState::parse(&format!("{HEADER}\n")),
            Some(FetchState::default())
        );
        for text in [
            String::new(),
            format!("quecto-board-fetch 2\nread {commit}\n"),
            format!("{HEADER}\nread {commit}\nread {commit}\n"),
            format!("{HEADER}\nomitted {blob}\nread {commit}\n"),
            format!("{HEADER}\nread HEAD\n"),
            format!("{HEADER}\nkept {blob}\n"),
        ] {
            assert_eq!(FetchState::parse(&text), None, "{text:?}");
        }
    }
}
