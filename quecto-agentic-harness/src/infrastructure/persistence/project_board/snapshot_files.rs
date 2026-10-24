//! The files a board snapshot is (epic #2705 B4): each artifact's file,
//! written by its kind's codec, at the path a read of the board finds it
//! (`tasks/<id>.md`, `plans/<id>.<n>.md`, ...). The board server's
//! snapshot answers them; a store that keeps the markdown itself keeps
//! the same bytes.
use super::artifacts::file_of;
use super::board_tree::{TASKS, folder_file, task_file};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardFile, BoardSnapshot, SnapshotFiles,
};

/// [`SnapshotFiles`] by the board's markdown codecs.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownSnapshotFiles;

impl SnapshotFiles for MarkdownSnapshotFiles {
    fn files(&self, snapshot: &BoardSnapshot) -> Result<Vec<BoardFile>, String> {
        let artifacts = snapshot
            .tasks
            .iter()
            .cloned()
            .map(BoardArtifact::Task)
            .chain(snapshot.plans.iter().cloned().map(BoardArtifact::Plan))
            .chain(snapshot.runs.iter().cloned().map(BoardArtifact::Run))
            .chain(snapshot.changes.iter().cloned().map(BoardArtifact::Change))
            .chain(snapshot.reviews.iter().cloned().map(BoardArtifact::Review))
            .chain(
                snapshot
                    .discussions
                    .iter()
                    .cloned()
                    .map(BoardArtifact::Discussion),
            );
        let mut files = artifacts
            .map(|artifact| file(&artifact))
            .collect::<Result<Vec<_>, _>>()?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}

/// `artifact`'s file, where a read finds it.
fn file(artifact: &BoardArtifact) -> Result<BoardFile, String> {
    let bytes = file_of(artifact)?;
    let path = match artifact.key() {
        ArtifactKey::Task(id) => format!("{TASKS}{}", task_file(&id)),
        key => {
            let placed = folder_file(&key, bytes.clone())?;
            format!("{}/{}", placed.folder, placed.name)
        }
    };
    Ok(BoardFile { path, bytes })
}
