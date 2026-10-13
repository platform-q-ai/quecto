//! What a write makes of the board's folders besides `tasks/` (#2481
//! S10a): each artifact's file in its kind's folder, named by its key as a
//! read spells it, and every folder as the write leaves it: carried as the
//! very tree it was read as, or rebuilt from what it carries, by object,
//! with the files written in place and every entry a write drops gone.
//!
//! A folder holding a name listed more than once (S10b) holds only itself:
//! a write that writes nothing in it carries it as the very tree it was
//! read as, so no write makes a tree of the copies (git rejects one, and
//! the remote already holds this one), and a write into it is refused until
//! a clear names the copy each such name keeps.
use super::folders::{FOLDERS, Folder};
use super::{ARCHIVE, Board, CarriedTree, TASKS, TREE, named_twice};
use crate::application::project_board::ports::ArtifactKey;
use std::collections::BTreeMap;

/// One file a write makes in a folder besides `tasks/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFile {
    pub folder: &'static str,
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The file of `key`'s artifact, `bytes`, where a read finds it: its
/// kind's folder, under the name that spells `key` there.
///
/// # Errors
/// A task's key (its file lies in `tasks/`), or a file a read would set
/// aside: a name that spells another key or none, or past its kind's cap.
pub fn folder_file(key: &ArtifactKey, bytes: Vec<u8>) -> Result<FolderFile, String> {
    let numbered = |id: &crate::domain::project_board::value_objects::slug::Slug, n: u32| {
        format!("{}.{n}.md", id.as_str())
    };
    let (folder, name) = match key {
        ArtifactKey::Task(_) => return Err(format!("{key}: a task's file lies in {TASKS}")),
        ArtifactKey::Plan(id, n) => ("plans", numbered(id, *n)),
        ArtifactKey::Run(id, n) => ("runs", numbered(id, *n)),
        ArtifactKey::Change(id, n) => ("changes", numbered(id, *n)),
        ArtifactKey::Review(id, n) => ("reviews", numbered(id, *n)),
        ArtifactKey::Discussion(id) => ("discussions", format!("{}.md", id.as_str())),
    };
    let found = FOLDERS.iter().find(|found| found.naming.folder == folder);
    let Some(found) = found else {
        return Err(format!("{key}: no folder of this quecto keeps it"));
    };
    // The read's own rules: a file it would set aside is never written.
    let spelled = name.strip_suffix(".md").and_then(found.spell);
    match (
        spelled.as_ref() == Some(key),
        name.len() <= found.naming.max_name,
        bytes.len() <= found.naming.file_bytes,
    ) {
        (true, true, true) => Ok(FolderFile {
            folder,
            name,
            bytes,
        }),
        (false, _, _) | (_, false, _) => Err(format!("{key}: has no file name the board can read")),
        (true, true, false) => Err(format!(
            "{key}: its file would be {} bytes; the most is {}",
            bytes.len(),
            found.naming.file_bytes
        )),
    }
}

/// A folder besides `tasks/` as a write leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderAfter<'a> {
    /// As it was read: the very tree, which drops nothing.
    Carried(&'a CarriedTree),
    /// Rebuilt from what it carries (`<mode> <type> <oid>` by name) and the
    /// files written in it, with what a read reads of them.
    Rebuilt {
        carried: BTreeMap<String, String>,
        written: Vec<&'a FolderFile>,
        bytes: usize,
    },
}

impl FolderAfter<'_> {
    pub fn entries(&self) -> usize {
        match self {
            Self::Carried(tree) => tree.entries,
            Self::Rebuilt {
                carried, written, ..
            } => carried.len() + written.len(),
        }
    }

    pub fn bytes(&self) -> usize {
        match self {
            Self::Carried(tree) => tree.bytes,
            Self::Rebuilt { bytes, .. } => *bytes,
        }
    }
}

impl Board {
    /// Every folder besides `tasks/` as a write of `written` leaves it, by
    /// name: one it writes in, or one holding an entry a write drops, is
    /// rebuilt; any other is carried as it was read. A folder never read,
    /// and written in, is made.
    pub fn folders_after<'a>(
        &'a self,
        written: &'a [FolderFile],
    ) -> BTreeMap<&'static str, FolderAfter<'a>> {
        FOLDERS
            .iter()
            .filter_map(|folder| self.folder_after(folder, written))
            .collect()
    }

    fn folder_after<'a>(
        &'a self,
        folder: &Folder,
        written: &'a [FolderFile],
    ) -> Option<(&'static str, FolderAfter<'a>)> {
        let name = folder.naming.folder;
        let mine: Vec<&FolderFile> = written.iter().filter(|file| file.folder == name).collect();
        let read = self.folders.get(name);
        // A write into a folder with a name listed twice is refused first.
        assert!(
            mine.is_empty() || read.is_none_or(|tree| tree.repeated.is_empty()),
            "no write rebuilds a folder with a name listed twice"
        );
        let after = match (read, mine.is_empty()) {
            (None, true) => return None,
            (Some(tree), true) if tree.carries_every_entry() || !tree.repeated.is_empty() => {
                FolderAfter::Carried(tree)
            }
            (read, _) => {
                let empty = CarriedTree::default();
                let tree = read.unwrap_or(&empty);
                let mut carried = tree.files.clone();
                for file in &mine {
                    carried.remove(&file.name);
                }
                let kept: usize = carried.keys().filter_map(|name| tree.sizes.get(name)).sum();
                let bytes = kept + mine.iter().map(|file| file.bytes.len()).sum::<usize>();
                FolderAfter::Rebuilt {
                    carried,
                    written: mine,
                    bytes,
                }
            }
        };
        Some((name, after))
    }

    /// Why a write of `written` at board commit `head` is refused: a file
    /// it writes in a folder besides `tasks/` that lists a name more than
    /// once (the first such folder's, in the plan's order).
    pub fn folder_repeated_refusal(&self, head: &str, written: &[FolderFile]) -> Option<String> {
        FOLDERS.iter().find_map(|folder| {
            let name = folder.naming.folder;
            let (path, copies) = self.folders.get(name)?.repeated.first()?;
            written.iter().any(|file| file.folder == name).then(|| {
                let writes = format!("no write in {name}/");
                named_twice(&format!("{name}/{path}"), head, copies, &writes)
            })
        })
    }

    /// The archive's root entry, carried as it was read, if the board has
    /// one.
    pub fn archive_entry(&self) -> Option<(String, String)> {
        let oid = self.archive.as_ref()?;
        Some((ARCHIVE.to_string(), format!("{TREE} tree {oid}")))
    }
}
