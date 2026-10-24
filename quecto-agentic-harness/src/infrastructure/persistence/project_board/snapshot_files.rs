//! The files a git board's snapshot is, byte for byte (epic #2705 B4):
//! each artifact read, by the blob at its path at the snapshot's head, and
//! each file set aside, by its object; never a re-encoding. Sizes are read
//! first and their sum over the paths served held to the bound.
use std::collections::BTreeMap;

use super::board_tree::{FILTER_BYTES, MAX_BOARD_BYTES, folder_file, object_id, task_file, told};
use super::cat_file::{Found, contents, objects};
use super::git::Git;
use crate::application::project_board::ports::{
    ArtifactKey, BoardFile, BoardSnapshot, SnapshotFiles, SnapshotFilesError,
};

/// The most stored bytes one snapshot serves: what one board read takes.
pub const MAX_SNAPSHOT_BYTES: usize = MAX_BOARD_BYTES;

/// [`SnapshotFiles`] by the blobs of the git store's repository.
#[derive(Debug, Clone)]
pub struct GitSnapshotFiles {
    git: Git,
    most: usize,
}

impl GitSnapshotFiles {
    pub fn new(git: Git) -> Self {
        let most = MAX_SNAPSHOT_BYTES;
        Self { git, most }
    }

    /// Serving at most `most` stored bytes (never past the bound).
    #[must_use]
    pub fn bounded(mut self, most: usize) -> Self {
        self.most = most.min(MAX_SNAPSHOT_BYTES);
        self
    }
}

/// Where a read finds `key`'s file.
fn path_of(key: &ArtifactKey) -> Result<String, String> {
    match key {
        ArtifactKey::Task(id) => Ok(format!("tasks/{}", task_file(id))),
        other => {
            folder_file(other, Vec::new()).map(|file| format!("{}/{}", file.folder, file.name))
        }
    }
}

impl SnapshotFiles for GitSnapshotFiles {
    fn files(&self, snapshot: &BoardSnapshot) -> Result<Vec<BoardFile>, SnapshotFilesError> {
        let unreadable = SnapshotFilesError::Unreadable;
        let failed = |error| unreadable(told(&error).into_owned());
        let head = object_id(&snapshot.head.0).map_err(failed)?;
        let paths = snapshot
            .tasks
            .iter()
            .map(ArtifactKey::of_task)
            .chain(snapshot.plans.iter().map(ArtifactKey::of_plan))
            .chain(snapshot.runs.iter().map(ArtifactKey::of_run))
            .chain(snapshot.changes.iter().map(ArtifactKey::of_change))
            .chain(snapshot.reviews.iter().map(ArtifactKey::of_review))
            .chain(snapshot.discussions.iter().map(ArtifactKey::of_discussion))
            .map(|key| path_of(&key))
            .collect::<Result<Vec<_>, _>>()
            .map_err(unreadable)?;
        // Each artifact's object at the head, in order, by one rev-parse.
        let names: Vec<String> = paths.iter().map(|path| format!("{head}:{path}")).collect();
        let mut args = vec!["rev-parse"];
        args.extend(names.iter().map(String::as_str));
        let resolved = match names.len() {
            0 => Ok(String::new()),
            n => self
                .git
                .ok_up_to(&args, None, n * 66)
                .map(|out| String::from_utf8_lossy(&out).into()),
        }
        .map_err(|error| unreadable(format!("the board's files at {head}: {error}")))?;
        let oids: Vec<&str> = resolved.lines().collect();
        match oids.len() == names.len() {
            true => {}
            false => {
                return Err(unreadable(format!(
                    "rev-parse named {} of {} files",
                    oids.len(),
                    names.len()
                )));
            }
        }
        let mut wanted: Vec<(String, String, Option<String>)> = Vec::new();
        for (path, oid) in paths.into_iter().zip(&oids) {
            wanted.push((path, object_id(oid).map_err(failed)?, None));
        }
        for file in &snapshot.quarantined {
            let why = Some(file.reason.clone());
            wanted.push((
                file.path.clone(),
                object_id(&file.object).map_err(failed)?,
                why,
            ));
        }
        let found =
            objects(&self.git, wanted.iter().map(|(_, oid, _)| oid.as_str())).map_err(failed)?;
        // The bound sums over paths (a shared blob counts per path). A
        // set-aside blob past every read cap is served as none, as a
        // partial clone (which never fetches it) must.
        let (mut total, mut plan) = (0usize, Vec::with_capacity(wanted.len()));
        for (path, oid, set_aside) in wanted {
            let too_large = |why: &str, size: String| format!("{why}; too large to serve: {size}");
            let (read, set_aside) = match (set_aside, found.get(&oid)) {
                (None, Some(Found::Present(blob))) if blob.kind == "blob" => {
                    (Some(blob.size), None)
                }
                (None, _) => return Err(unreadable(format!("{path} has no blob to read"))),
                (Some(why), Some(Found::Present(blob)))
                    if blob.kind == "blob" && blob.size < FILTER_BYTES =>
                {
                    (Some(blob.size), Some(why))
                }
                (Some(why), Some(Found::Present(blob))) if blob.kind == "blob" => {
                    (None, Some(too_large(&why, format!("{} bytes", blob.size))))
                }
                (Some(why), Some(Found::Present(_))) => (None, Some(why)),
                (Some(why), Some(Found::Missing) | None) => (
                    None,
                    Some(too_large(&why, format!("at least {FILTER_BYTES} bytes"))),
                ),
            };
            total = total.saturating_add(read.unwrap_or(0));
            plan.push((path, oid, read, set_aside));
        }
        match total <= self.most {
            true => {}
            false => {
                return Err(SnapshotFilesError::TooLarge(format!(
                    "the board's files are {total} bytes; the most one snapshot serves is {}",
                    self.most
                )));
            }
        }
        // Each blob read once, moved out on its last use, cloned before.
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
        let mut bytes = contents(&self.git, &read).map_err(failed)?;
        let mut files = Vec::with_capacity(plan.len());
        for (path, oid, read, set_aside) in plan {
            let held = match (read, uses.get_mut(&oid)) {
                (Some(_), Some((1, _))) => bytes.remove(&oid),
                (Some(_), Some((left, _))) => {
                    *left -= 1;
                    bytes.get(&oid).cloned()
                }
                (Some(_), None) | (None, _) => None,
            };
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
