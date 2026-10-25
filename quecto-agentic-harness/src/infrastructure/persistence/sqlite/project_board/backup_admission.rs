//! Bounded metadata admission, reusing the sole schema recognizer, not a history interpreter.
use super::super::{backup_sqlite::classify, storage};
use crate::application::project_board::ports::backup::BackupError;
use rusqlite::{Connection, OptionalExtension};

pub(super) fn admit(c: &Connection, board: &str) -> Result<(), BackupError> {
    storage::recognize_on(c, false).map_err(|_| BackupError::UnsupportedSchema)?;
    storage::current(c).map_err(|error| match error {
        crate::application::project_board::ports::BoardStoreError::NoBoard => BackupError::NoBoard,
        _ => BackupError::IdentityMismatch,
    })?;
    // SQL bounds are applied before allocating any untrusted text.
    type Meta = (
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let meta: Option<Meta> = c.query_row("SELECT board_id,algorithm,source_remote,source_ref,source_head,imported_count FROM history_meta WHERE id=1 AND (board_id IS NULL OR length(CAST(board_id AS BLOB))=16) AND length(CAST(algorithm AS BLOB))<=6 AND (source_remote IS NULL OR length(CAST(source_remote AS BLOB)) BETWEEN 1 AND 200) AND (source_ref IS NULL OR length(CAST(source_ref AS BLOB))<=64) AND (source_head IS NULL OR length(CAST(source_head AS BLOB))<=64)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(classify)?;
    let (binding, algorithm, remote, reference, head, count) =
        meta.ok_or(BackupError::IdentityMismatch)?;
    let native = binding.is_none()
        && algorithm == "sha256"
        && remote.is_none()
        && reference.is_none()
        && head.is_none()
        && count == 0;
    if native {
        let clean: bool = c.query_row("SELECT NOT EXISTS(SELECT 1 FROM git_objects) AND NOT EXISTS(SELECT 1 FROM revisions WHERE git_commit IS NOT NULL OR git_meta IS NOT NULL)", [], |r| r.get(0)).map_err(classify)?;
        if clean {
            return Ok(());
        }
    }
    let width = match algorithm.as_str() {
        "sha1" => Some(40),
        "sha256" => Some(64),
        _ => None,
    };
    if binding.as_deref() == Some(board)
        && (1..=4096).contains(&count)
        && remote.is_some()
        && reference.as_deref() == Some("refs/heads/quecto/board")
        && head.as_ref().is_some_and(|id| {
            Some(id.len()) == width
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        Ok(())
    } else {
        Err(BackupError::IdentityMismatch)
    }
}
