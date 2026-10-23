//! SPIKE (board service): [`ProjectBoardStore`] over a [`SqliteBoard`] in
//! this process (the server's own store, and the contract checks').
use super::files::{Head, artifact_path};
use super::sqlite::{RevisionMeta, SqliteBoard};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError,
    BoardWrite, ProjectBoardStore, SetAsideClear,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::infrastructure::persistence::project_board::artifacts::file_of;
use std::sync::Arc;

#[derive(Clone)]
pub struct SqliteBoardStore {
    board: Arc<SqliteBoard>,
    writer: Identity,
}

impl SqliteBoardStore {
    pub fn new(board: Arc<SqliteBoard>, writer: Identity) -> Self {
        Self { board, writer }
    }

    pub fn board(&self) -> &Arc<SqliteBoard> {
        &self.board
    }

    /// The writer recorded for the revision `head` names.
    pub fn committed_by(&self, head: &BoardHead) -> Option<Identity> {
        let head = Head::parse(&head.0)?;
        self.board.writer_of(head.revision).ok().flatten()
    }
}

/// `artifacts` as the files a write makes: each by its codec, at its path.
///
/// # Errors
/// What a codec refuses, as an invalid write.
pub fn encoded(
    artifacts: &[BoardArtifact],
) -> Result<Vec<(ArtifactKey, String, Vec<u8>)>, BoardStoreError> {
    artifacts
        .iter()
        .map(|artifact| {
            let bytes = file_of(artifact).map_err(BoardStoreError::InvalidWrite)?;
            let key = artifact.key();
            let path = artifact_path(&key, &bytes).map_err(BoardStoreError::InvalidWrite)?;
            Ok((key, path, bytes))
        })
        .collect()
}

impl ProjectBoardStore for SqliteBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        Ok(self.board.current()?.snapshot.clone())
    }

    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        // A write before a bootstrap is no board, whatever it holds.
        if self.board.head()?.is_none() {
            return Err(BoardStoreError::NoBoard);
        }
        let files = encoded(artifacts)?;
        let meta = RevisionMeta {
            write_id: Some(uuid::Uuid::new_v4().to_string()),
            ..RevisionMeta::by(self.writer.clone(), message, "write")
        };
        self.board.write_files(&expected.0, &files, &meta)
    }

    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let meta = RevisionMeta::by(self.writer.clone(), message, "clear");
        self.board.clear(&expected.0, clear, &meta)
    }

    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        self.board.bootstrap(&RevisionMeta::by(
            self.writer.clone(),
            "board: bootstrap",
            "bootstrap",
        ))
    }

    fn writer(&self) -> Identity {
        self.writer.clone()
    }
}
