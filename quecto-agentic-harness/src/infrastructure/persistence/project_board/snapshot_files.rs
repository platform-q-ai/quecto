//! The files a git board's snapshot is, byte for byte (epic #2705 B4):
//! each artifact read, by the blob its folder's tree holds at the
//! snapshot's head under the name a read spells for its key, and each
//! file set aside, by its object. The bytes are the stored blobs, never a
//! re-encoding: a legacy file the codec reads past is served as kept.
//!
//! Every blob's size is read first (`cat-file --batch-check`) and their
//! sum held to [`MAX_SNAPSHOT_BYTES`] before any is read.
use std::collections::{BTreeMap, BTreeSet};

use super::board_tree::{MAX_BOARD_BYTES, MAX_TREE_BYTES, folder_file, object_id, task_file, told};
use super::cat_file::{Found, contents, objects};
use super::git::Git;
use crate::application::project_board::ports::{
    ArtifactKey, BoardFile, BoardSnapshot, SnapshotFiles, SnapshotFilesError,
};

/// The most stored bytes one snapshot serves: what one read of the board
/// takes.
pub const MAX_SNAPSHOT_BYTES: usize = MAX_BOARD_BYTES;

/// The most one folder's `ls-tree` prints (as a read bounds it).
const MAX_FOLDER_LISTING: usize = 4 * MAX_TREE_BYTES;

/// [`SnapshotFiles`] by the blobs of the git store's repository.
#[derive(Debug, Clone)]
pub struct GitSnapshotFiles {
    git: Git,
    most: usize,
}

impl GitSnapshotFiles {
    /// Reads blobs through `git`, the store's repository (a reader).
    pub fn new(git: Git) -> Self {
        Self {
            git,
            most: MAX_SNAPSHOT_BYTES,
        }
    }

    /// This adapter, serving at most `most` stored bytes (a test's small
    /// bound); never more than [`MAX_SNAPSHOT_BYTES`].
    #[must_use]
    pub fn bounded(mut self, most: usize) -> Self {
        self.most = most.min(MAX_SNAPSHOT_BYTES);
        self
    }

    /// Each blob in `folder`'s tree at `head`, by name: `(type, object)`.
    fn folder(
        &self,
        head: &str,
        folder: &str,
    ) -> Result<BTreeMap<String, (String, String)>, String> {
        let tree = format!("{head}:{folder}");
        let listing = self
            .git
            .ok_up_to(&["ls-tree", "-z", &tree], None, MAX_FOLDER_LISTING)
            .map_err(|error| format!("{tree}: {error}"))?;
        let mut entries = BTreeMap::new();
        for row in listing
            .split(|byte| *byte == 0)
            .filter(|row| !row.is_empty())
        {
            let row = String::from_utf8_lossy(row);
            let parsed = row.split_once('\t').and_then(|(meta, name)| {
                match meta.split(' ').collect::<Vec<_>>().as_slice() {
                    [_mode, kind, oid] => {
                        Some((name.to_string(), (kind.to_string(), oid.to_string())))
                    }
                    _ => None,
                }
            });
            let Some((name, (kind, oid))) = parsed else {
                return Err(format!("unexpected ls-tree row {row:?}"));
            };
            entries.insert(
                name,
                (
                    kind,
                    object_id(&oid).map_err(|error| told(&error).into_owned())?,
                ),
            );
        }
        Ok(entries)
    }
}

/// Where a read finds `key`'s file: its folder and name.
fn placed(key: &ArtifactKey) -> Result<(&'static str, String), String> {
    match key {
        ArtifactKey::Task(id) => Ok(("tasks", task_file(id))),
        other => folder_file(other, Vec::new()).map(|file| (file.folder, file.name)),
    }
}

/// One file to serve: its path, its object (none for an entry with no
/// blob to read), and why it was set aside.
struct Wanted {
    path: String,
    object: String,
    set_aside: Option<String>,
}

impl SnapshotFiles for GitSnapshotFiles {
    fn files(&self, snapshot: &BoardSnapshot) -> Result<Vec<BoardFile>, SnapshotFilesError> {
        let unreadable = SnapshotFilesError::Unreadable;
        let head = object_id(&snapshot.head.0).map_err(|error| unreadable(told(&error).into()))?;
        let keys: Vec<ArtifactKey> = snapshot
            .tasks
            .iter()
            .map(ArtifactKey::of_task)
            .chain(snapshot.plans.iter().map(ArtifactKey::of_plan))
            .chain(snapshot.runs.iter().map(ArtifactKey::of_run))
            .chain(snapshot.changes.iter().map(ArtifactKey::of_change))
            .chain(snapshot.reviews.iter().map(ArtifactKey::of_review))
            .chain(snapshot.discussions.iter().map(ArtifactKey::of_discussion))
            .collect();
        let places = keys
            .iter()
            .map(placed)
            .collect::<Result<Vec<_>, _>>()
            .map_err(unreadable)?;
        let folders: BTreeSet<&str> = places.iter().map(|(folder, _)| *folder).collect();
        let mut trees = BTreeMap::new();
        for folder in folders {
            trees.insert(folder, self.folder(&head, folder).map_err(unreadable)?);
        }
        let mut wanted = Vec::new();
        for (folder, name) in places {
            let path = format!("{folder}/{name}");
            match trees.get(folder).and_then(|tree| tree.get(&name)) {
                Some((kind, oid)) if kind == "blob" => wanted.push(Wanted {
                    path,
                    object: oid.clone(),
                    set_aside: None,
                }),
                Some(_) | None => {
                    return Err(unreadable(format!(
                        "{path} is no file of the board at {head}"
                    )));
                }
            }
        }
        for file in &snapshot.quarantined {
            let object =
                object_id(&file.object).map_err(|error| unreadable(told(&error).into()))?;
            wanted.push(Wanted {
                path: file.path.clone(),
                object,
                set_aside: Some(file.reason.clone()),
            });
        }
        let found = objects(&self.git, wanted.iter().map(|file| file.object.as_str()))
            .map_err(|error| unreadable(told(&error).into()))?;
        let blobs: BTreeMap<&str, usize> = found
            .iter()
            .filter_map(|(oid, found)| match found {
                Found::Present(object) if object.kind == "blob" && !object.omitted => {
                    Some((oid.as_str(), object.size))
                }
                Found::Present(_) | Found::Missing => None,
            })
            .collect();
        let total = blobs
            .values()
            .fold(0usize, |sum, size| sum.saturating_add(*size));
        if total > self.most {
            return Err(SnapshotFilesError::TooLarge(format!(
                "the board's files are {total} bytes; the most one snapshot serves is {}",
                self.most
            )));
        }
        let read: Vec<(&str, usize)> = blobs.iter().map(|(oid, size)| (*oid, *size)).collect();
        let bytes = contents(&self.git, &read).map_err(|error| unreadable(told(&error).into()))?;
        let mut files = Vec::with_capacity(wanted.len());
        for file in wanted {
            let held = bytes.get(&file.object).cloned();
            // An artifact read always has its blob; only an entry set
            // aside may hold none.
            if held.is_none() && file.set_aside.is_none() {
                return Err(unreadable(format!("{} has no blob to read", file.path)));
            }
            files.push(BoardFile {
                path: file.path,
                bytes: held,
                set_aside: file.set_aside,
            });
        }
        files.sort_by(|a, b| (&a.path, &a.set_aside).cmp(&(&b.path, &b.set_aside)));
        Ok(files)
    }
}
