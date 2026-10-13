//! Writing artifacts of every kind (#2481 S10a): each as its kind's
//! canonical file (`artifacts::file_of`), tasks in `tasks/` and the rest
//! in their folders (`board_writes`), in one commit, every other file
//! carried forward by its object.
//!
//! A write always writes what it is given: whether a file changed is the
//! store's to see by its bytes (git names equal bytes by one object), never
//! by comparing values, whose equality may be looser than their files (a
//! repository's name ignores case).
use super::*;
use crate::application::project_board::ports::ArtifactKey;
use crate::infrastructure::persistence::project_board::artifacts::file_of;
use crate::infrastructure::persistence::project_board::board_tree::{
    FolderAfter, FolderFile, folder_file,
};

impl GitBoardStore {
    pub(super) fn write_artifacts(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let invalid = |why: String| BoardStoreError::InvalidWrite(why);
        line("message", message, 200).map_err(|error| invalid(error.to_string()))?;
        let keys: BTreeSet<ArtifactKey> = artifacts.iter().map(BoardArtifact::key).collect();
        if keys.len() != artifacts.len() || artifacts.is_empty() {
            return Err(invalid("write one or more board files, each once".into()));
        }
        let base = self.base(expected)?;
        let board = self.board_at(&base)?;
        if let Some(refusal) = board.repeated_refusal(&base) {
            return Ok(BoardWrite::Refused(refusal));
        }
        if let Some(refusal) = board.folders_repeated_refusal() {
            return Err(BoardStoreError::FolderSetAside(refusal));
        }
        let set_aside = board
            .quarantined
            .iter()
            .find(|file| file.key.as_ref().is_some_and(|key| keys.contains(key)));
        if let Some(file) = set_aside {
            return Ok(BoardWrite::Refused(format!(
                "{} is set aside: {}",
                file.path, file.reason
            )));
        }
        let (tasks, files) = encoded(artifacts).map_err(invalid)?;
        let folders = board.folders_after(&files);
        // A read takes at most so much, so no write makes a board that
        // cannot be read.
        board.within_bounds(&tasks, &folders, self.limits)?;
        let tasks = self.tasks_tree(&board, &tasks)?;
        let folders = self.folder_trees(&folders)?;
        self.land(&base, &board, &tasks, folders, message)
    }

    /// The root entries of the folders besides `tasks/` as a write leaves
    /// them: a folder carried as it was read by its tree, one rebuilt by a
    /// new tree of what it carries and the files written in it. A folder
    /// left with nothing in it is no folder.
    pub(super) fn folder_trees(
        &self,
        folders: &BTreeMap<&'static str, FolderAfter<'_>>,
    ) -> Result<BTreeMap<String, String>, BoardStoreError> {
        let mut trees = BTreeMap::new();
        for (name, after) in folders {
            let oid = match after {
                FolderAfter::Carried(tree) => tree.oid.clone(),
                FolderAfter::Rebuilt { written, .. } if after.entries() == 0 => {
                    assert!(written.is_empty(), "a folder written in holds its file");
                    continue;
                }
                FolderAfter::Rebuilt {
                    carried, written, ..
                } => {
                    let mut files = carried.clone();
                    for file in written {
                        let object = self.blob(&file.bytes)?;
                        let entry = format!("{REGULAR_FILE} blob {object}");
                        let replaced = files.insert(file.name.clone(), entry);
                        assert!(
                            replaced.is_none(),
                            "a file written replaces none it carries"
                        );
                    }
                    assert_eq!(files.len(), after.entries(), "each file is one entry");
                    self.tree(&files)?
                }
            };
            trees.insert((*name).to_string(), format!("{TREE} tree {oid}"));
        }
        Ok(trees)
    }
}

/// The files a write makes: each task's, and each other kind's in its
/// folder.
type Encoded<'a> = (Vec<(&'a Task, Vec<u8>)>, Vec<FolderFile>);

/// `artifacts` as the files a write makes of them.
///
/// # Errors
/// What a codec refuses, named by its key, field and line.
fn encoded(artifacts: &[BoardArtifact]) -> Result<Encoded<'_>, String> {
    let (mut tasks, mut files) = (vec![], vec![]);
    for artifact in artifacts {
        let bytes = file_of(artifact)?;
        match artifact {
            BoardArtifact::Task(task) => tasks.push((task, bytes)),
            BoardArtifact::Plan(_)
            | BoardArtifact::Run(_)
            | BoardArtifact::Change(_)
            | BoardArtifact::Review(_)
            | BoardArtifact::Discussion(_) => files.push(folder_file(&artifact.key(), bytes)?),
        }
    }
    Ok((tasks, files))
}
