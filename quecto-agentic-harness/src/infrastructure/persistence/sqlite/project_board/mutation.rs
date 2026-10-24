//! One ordinary IMMEDIATE transaction: CAS, admission, persistence, provenance.
use super::*;
use crate::domain::project_board::entities::task::Task;
use crate::infrastructure::persistence::project_board::{
    board_tree::{FORMAT, FORMAT_FILE, FolderAfter, FolderFile, MAX_READ_ENTRIES, task_file},
    store::{README, cleared, encoded},
};

impl SqliteProjectBoardStore {
    pub(super) fn change(
        &self,
        expected: &BoardHead,
        artifacts: Option<&[BoardArtifact]>,
        clear: Option<&SetAsideClear>,
    ) -> Result<BoardWrite, BoardStoreError> {
        object_id(&expected.0).map_err(|_| invalid("not a board head"))?;
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = storage::Scope::begin(&mut c, true)?;
        let (current, root) = storage::current(&tx)?;
        let known: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM revisions WHERE head=?1)",
                [&expected.0],
                |r| r.get(0),
            )
            .map_err(unavailable)?;
        match (known, current == *expected) {
            (true, true) => {}
            (true, false) => return Ok(BoardWrite::Lost),
            (false, _) => return Err(invalid("not a board head")),
        }
        let before = self.board(&tx, &root)?;
        let after;
        let (tasks, files) = match (artifacts, clear) {
            (Some(artifacts), None) => {
                if let Some(refusal) = before.repeated_refusal(&current.0) {
                    return Ok(BoardWrite::Refused(refusal));
                }
                let (tasks, files) = encoded(artifacts).map_err(invalid)?;
                if let Some(refusal) = before.folder_repeated_refusal(&current.0, &files) {
                    return Ok(BoardWrite::Refused(refusal));
                }
                let keys: BTreeSet<_> = artifacts.iter().map(BoardArtifact::key).collect();
                if let Some(file) = before.quarantined.iter().find(|file| {
                    file.carried && file.key.as_ref().is_some_and(|key| keys.contains(key))
                }) {
                    return Ok(BoardWrite::Refused(format!(
                        "{} is set aside: {}",
                        file.path, file.reason
                    )));
                }
                before.within_bounds(&tasks, &before.folders_after(&files), LIMITS)?;
                after = before;
                (tasks, files)
            }
            (None, Some(clear)) => {
                let (cleared, keeps) = match cleared(&before, clear) {
                    Ok(result) => result,
                    Err(refusal) => return Ok(BoardWrite::Refused(refusal)),
                };
                cleared.within_clear_bounds(
                    &before,
                    &cleared.folders_after(&[]),
                    LIMITS,
                    &keeps,
                )?;
                after = cleared;
                (vec![], vec![])
            }
            _ => unreachable!("exactly one mutation kind"),
        };
        // Fixture state exists only in the feature build, is fixture-owned,
        // and is consumed only after every input/CAS/prospective guard succeeds.
        #[cfg(feature = "test-support")]
        let fault = self.hooks.take()?;
        #[cfg(feature = "test-support")]
        if matches!(fault, Some(fixture::Fault::Refuse)) {
            return Ok(BoardWrite::Refused("fixture refusal".into()));
        }
        let writer = storage::ObjectWriter::new(&tx);
        #[cfg(feature = "test-support")]
        let writer = writer.with_fault(fault);
        let mut writer = writer;
        let root = Self::assemble(&mut writer, &after, &tasks, &files)?;
        let next = writer.advance(&root, &self.writer, self.principal.as_deref())?;
        tx.commit()?;
        Ok(BoardWrite::Written(next))
    }
    fn assemble(
        writer: &mut storage::ObjectWriter<'_>,
        board: &Board,
        written: &[(&Task, Vec<u8>)],
        files: &[FolderFile],
    ) -> Result<String, BoardStoreError> {
        let mut tasks = board.kept.clone();
        let ids: BTreeSet<_> = written.iter().map(|(task, _)| &task.fields().id).collect();
        for (id, held) in &board.tasks {
            if ids.contains(id) {
                continue;
            }
            let replaced = tasks.insert(task_file(id), format!("100644 blob {}", held.object));
            assert!(replaced.is_none(), "a read task is never kept");
        }
        for (task, bytes) in written {
            let oid = writer.blob(bytes)?;
            tasks.insert(task_file(&task.fields().id), format!("100644 blob {oid}"));
        }
        assert!(
            tasks.len() <= MAX_READ_ENTRIES,
            "prospective task bounds checked before persistence"
        );
        let tasks = writer.mapped_tree(&tasks)?;
        let mut folders = BTreeMap::new();
        for (name, after) in board.folders_after(files) {
            let oid = match &after {
                FolderAfter::Carried(tree) => tree.oid.clone(),
                FolderAfter::Rebuilt { written, .. } if after.entries() == 0 => {
                    assert!(written.is_empty());
                    continue;
                }
                FolderAfter::Rebuilt {
                    carried, written, ..
                } => {
                    let mut entries = carried.clone();
                    for file in written {
                        let oid = writer.blob(&file.bytes)?;
                        let replaced =
                            entries.insert(file.name.clone(), format!("100644 blob {oid}"));
                        assert!(
                            replaced.is_none(),
                            "a written file replaces none it carries"
                        );
                    }
                    assert_eq!(entries.len(), after.entries());
                    writer.mapped_tree(&entries)?
                }
            };
            folders.insert(name.to_string(), format!("040000 tree {oid}"));
        }
        for (name, tree) in &board.folders {
            assert!(
                tree.repeated.is_empty()
                    || folders.get(*name) == Some(&format!("040000 tree {}", tree.oid)),
                "a repeated folder must be carried by identity"
            );
        }
        let readme = writer.blob(README)?;
        let marker = writer.blob(FORMAT)?;
        let format = writer.mapped_tree(&BTreeMap::from([(
            FORMAT_FILE.into(),
            format!("100644 blob {marker}"),
        )]))?;
        let mut roots = BTreeMap::from([
            ("README.md".into(), format!("100644 blob {readme}")),
            ("format".into(), format!("040000 tree {format}")),
            ("tasks".into(), format!("040000 tree {tasks}")),
        ]);
        let carried: BTreeMap<_, _> = folders.into_iter().chain(board.archive_entry()).collect();
        let count = roots.len() + carried.len();
        roots.extend(carried);
        assert_eq!(roots.len(), count, "root and carried names are disjoint");
        writer.mapped_tree(&roots)
    }
}
