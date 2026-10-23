//! Canonical folder interpretation for flat, regular-file persistence adapters.
use super::*;
use crate::application::project_board::ports::ArtifactKey;
use crate::infrastructure::persistence::project_board::artifacts::{decode, task::TaskCodec};
use entries::{Disposition, TASK_NAMING, TaskFile, disposition, task_name};

/// Interpret durable regular files with the same naming, codecs and bounds as git.
/// Paths are unique in flat stores: duplicate names cannot be represented.
pub(crate) fn from_rows(rows: &[(String, String, Vec<u8>)]) -> Result<Board, BoardStoreError> {
    let mut board = Board::default();
    for (path, oid, bytes) in rows {
        let (folder, name) = path
            .split_once('/')
            .ok_or_else(|| BoardStoreError::Clash("invalid board path".into()))?;
        let found = FOLDERS.iter().find(|f| f.naming.folder == folder);
        let (naming, file) = match (folder, found) {
            ("tasks", _) => (
                TASK_NAMING,
                task_name(name).map(|(id, kind)| (id.map(ArtifactKey::Task), kind)),
            ),
            (_, Some(f)) => (
                f.naming,
                name.strip_suffix(".md")
                    .map(|stem| ((f.spell)(stem), TaskFile::Markdown)),
            ),
            _ => return Err(BoardStoreError::Clash("unknown board folder".into())),
        };
        let entry = Entry {
            name: name.into(),
            raw: name.as_bytes().to_vec(),
            mode: REGULAR_FILE.into(),
            oid: oid.clone(),
        };
        let object = Object {
            kind: "blob".into(),
            size: bytes.len(),
            omitted: false,
        };
        let (key, reason, carried, read) =
            match disposition(&entry, Some(&object), 1, file, &naming) {
                Disposition::Read(key) => {
                    let decoded = match found {
                        Some(f) => (f.decode)(bytes),
                        None => decode::<TaskCodec>(bytes).map(BoardArtifact::Task),
                    };
                    match decoded
                        .map_err(|e| format!("{} file: {e}", naming.noun))
                        .and_then(|a| match a.key() == key {
                            true => Ok(a),
                            false => Err(format!("holds {}, not {key}", a.key())),
                        }) {
                        Ok(BoardArtifact::Task(task)) => {
                            board.tasks.insert(
                                task.fields().id.clone(),
                                HeldTask {
                                    task,
                                    object: oid.clone(),
                                },
                            );
                            (None, None, true, true)
                        }
                        Ok(artifact) => {
                            board.artifacts.push(artifact);
                            (None, None, true, true)
                        }
                        Err(reason) => (Some(key), Some(reason), true, true),
                    }
                }
                Disposition::Aside { id, reason, kept } => (id, Some(reason), kept, false),
            };
        if let Some(reason) = reason {
            board.quarantined.push(QuarantinedFile {
                path: entry_path(folder, name.as_bytes(), oid),
                key,
                reason: capped(&sanitised(reason.as_bytes())),
                object: oid.clone(),
                carried,
            });
        }
        let value = format!("{REGULAR_FILE} blob {oid}");
        match found {
            None => {
                board.listed += 1;
                let decoded = board.tasks.values().any(|held| {
                    held.object == *oid && format!("{}.md", held.task.fields().id.as_str()) == name
                });
                if let (true, false) = (carried, decoded) {
                    board.kept.insert(name.into(), value);
                }
                if read {
                    board.read_bytes.insert(name.into(), bytes.len());
                }
            }
            Some(f) => {
                let tree = board.folders.entry(f.naming.folder).or_default();
                tree.entries += 1;
                if carried {
                    tree.files.insert(name.into(), value);
                }
                if read {
                    tree.sizes.insert(name.into(), bytes.len());
                    tree.bytes += bytes.len();
                }
                if tree.entries > f.max_entries || tree.bytes > f.max_bytes {
                    return Err(BoardStoreError::TooLarge(format!(
                        "{folder}/ exceeds read bounds"
                    )));
                }
            }
        }
    }
    if board.listed > MAX_READ_ENTRIES
        || board.read_bytes.values().sum::<usize>() > LIMITS.tasks_bytes
    {
        return Err(BoardStoreError::TooLarge(
            "tasks/ exceeds read bounds".into(),
        ));
    }
    let entries = board.listed + board.folders.values().map(|t| t.entries).sum::<usize>();
    let bytes = board.read_bytes.values().sum::<usize>()
        + board.folders.values().map(|t| t.bytes).sum::<usize>();
    bounds::within_read(entries, bytes, LIMITS).map_err(|e| match e {
        Unread::Failed(e) => e,
        Unread::Missing(_) => unreachable!("flat files are local"),
    })?;
    board.quarantined.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(board)
}
