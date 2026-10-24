//! The files a git board's snapshot is, byte for byte (epic #2705 B4):
//! each artifact read, by the blob its folder's tree holds at the
//! snapshot's head under the name a read spells for its key, and each
//! file set aside, by its object. The bytes are the stored blobs, never a
//! re-encoding: a legacy file the codec reads past is served as kept.
//!
//! Every blob's size is read first (`cat-file --batch-check`) and their
//! sum held to [`MAX_SNAPSHOT_BYTES`] before any is read.
use std::collections::{BTreeMap, BTreeSet};

use super::board_tree::{
    FILTER_BYTES, MAX_BOARD_BYTES, MAX_TREE_BYTES, folder_file, object_id, task_file, told,
};
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
        // Each path is served whole, so the bound is the sum over paths (a
        // blob two paths share counts twice). A set-aside blob past every
        // read cap is served as none, as a partial clone (which never
        // fetched it) must.
        let (mut total, mut plan) = (0usize, Vec::with_capacity(wanted.len()));
        for file in wanted {
            let too_large = |why: &str, size: String| format!("{why}; too large to serve: {size}");
            let (read, set_aside) = match (file.set_aside, found.get(&file.object)) {
                (None, Some(Found::Present(blob))) if blob.kind == "blob" => {
                    (Some(blob.size), None)
                }
                (None, _) => return Err(unreadable(format!("{} has no blob to read", file.path))),
                (Some(why), Some(Found::Present(blob)))
                    if blob.kind == "blob" && blob.size < FILTER_BYTES =>
                {
                    (Some(blob.size), Some(why))
                }
                (Some(why), Some(Found::Present(blob))) if blob.kind == "blob" => {
                    (None, Some(too_large(&why, format!("{} bytes", blob.size))))
                }
                (Some(why), Some(Found::Present(_))) => (None, Some(why)),
                (Some(why), Some(Found::Missing) | None) => {
                    let least = format!("at least {FILTER_BYTES} bytes");
                    (None, Some(too_large(&why, least)))
                }
            };
            total = total.saturating_add(read.unwrap_or(0));
            plan.push((file.path, file.object, read, set_aside));
        }
        if total > self.most {
            return Err(SnapshotFilesError::TooLarge(format!(
                "the board's files are {total} bytes; the most one snapshot serves is {}",
                self.most
            )));
        }
        let mut uses: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for (_, oid, read, _) in &plan {
            if let Some(size) = read {
                uses.entry(oid.clone()).or_insert((0, *size)).0 += 1;
            }
        }
        let read: Vec<(&str, usize)> = uses
            .iter()
            .map(|(oid, (_, size))| (oid.as_str(), *size))
            .collect();
        let mut bytes =
            contents(&self.git, &read).map_err(|error| unreadable(told(&error).into()))?;
        let mut files = Vec::with_capacity(plan.len());
        for (path, oid, read, set_aside) in plan {
            // Moved out on its last use, so no blob is held twice for long.
            let held = match (read, uses.get_mut(&oid)) {
                (Some(_), Some((1, _))) => bytes.remove(&oid),
                (Some(_), Some((left, _))) => {
                    *left -= 1;
                    bytes.get(&oid).cloned()
                }
                (Some(_), None) | (None, _) => None,
            };
            match (read, &held) {
                (Some(_), Some(_)) | (None, None) => {}
                (Some(_), None) | (None, Some(_)) => {
                    return Err(unreadable(format!("cat-file did not give {path}")));
                }
            }
            files.push(BoardFile {
                path,
                bytes: held,
                set_aside,
            });
        }
        files.sort_by(|a, b| (&a.path, &a.set_aside).cmp(&(&b.path, &b.set_aside)));
        Ok(files)
    }
}
