//! Feature-only structural fixture editor and fixture-owned one-shot faults.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::Reader;
use std::sync::Arc;

#[derive(Default)]
pub struct FixtureHooks(Mutex<Option<Fault>>);
#[derive(Clone, Copy)]
pub(super) enum Fault {
    Refuse,
    AfterFirstRow,
}
impl FixtureHooks {
    pub(super) fn take(&self) -> Result<Option<Fault>, BoardStoreError> {
        Ok(self.0.lock().map_err(unavailable)?.take())
    }

    fn arm(&self, fault: Fault) -> Result<(), BoardStoreError> {
        let mut pending = self.0.lock().map_err(unavailable)?;
        match pending.is_none() {
            true => {
                *pending = Some(fault);
                Ok(())
            }
            false => Err(unavailable("fixture fault already pending")),
        }
    }
}
/// Raw names and ordered duplicates are intentionally not artifact keys.
#[derive(Clone, Debug)]
pub struct RawEntry {
    pub mode: String,
    pub name: Vec<u8>,
    pub oid: String,
}
pub struct RawEditor<'a> {
    store: &'a SqliteProjectBoardStore,
    connection: &'a Connection,
}
impl RawEditor<'_> {
    pub fn current(&self) -> Result<(BoardHead, String), BoardStoreError> {
        storage::current(self.connection)
    }
    pub fn blob(&self, bytes: &[u8]) -> Result<String, BoardStoreError> {
        storage::blob(self.connection, bytes)
    }
    pub fn tree(&self, rows: &[RawEntry]) -> Result<String, BoardStoreError> {
        let entries: Vec<_> = rows
            .iter()
            .map(|row| storage::entry(&row.mode, &row.name, row.oid.clone()))
            .collect();
        storage::tree(self.connection, &entries)
    }
    pub fn entries(&self, tree: &str) -> Result<Vec<RawEntry>, BoardStoreError> {
        self.store
            .reader(self.connection)
            .listing(
                tree,
                4 * super::super::super::project_board::board_tree::MAX_TREE_BYTES,
            )
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|row| RawEntry {
                        mode: row.mode,
                        name: row.raw,
                        oid: row.oid,
                    })
                    .collect()
            })
            .map_err(|e| match e {
                super::super::super::project_board::board_tree::Unread::Failed(e) => e,
                _ => unavailable("missing fixture tree"),
            })
    }
}
impl SqliteProjectBoardStore {
    pub fn open_with_hooks(
        path: &Path,
        writer: Identity,
        hooks: Arc<FixtureHooks>,
    ) -> Result<Self, BoardStoreError> {
        let mut store = Self::open(path, writer)?;
        store.hooks = hooks;
        Ok(store)
    }
    pub fn fixture_edit(
        &self,
        edit: impl FnOnce(&mut RawEditor<'_>) -> Result<String, BoardStoreError>,
    ) -> Result<BoardHead, BoardStoreError> {
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        storage::current(&tx)?;
        let root = edit(&mut RawEditor {
            store: self,
            connection: &tx,
        })?;
        let head = storage::advance_message(&tx, &root, &self.writer, "board: fixture edit")?;
        tx.commit().map_err(unavailable)?;
        Ok(head)
    }
    pub fn fixture_put_raw(&self, path: &str, bytes: &[u8]) -> Result<(), BoardStoreError> {
        let (folder, name) = path
            .split_once('/')
            .ok_or_else(|| invalid("fixture path needs folder/name"))?;
        self.fixture_edit(|editor| {
            let (_, root) = editor.current()?;
            let mut roots = editor.entries(&root)?;
            let mut files = match roots.iter().find(|r| r.name == folder.as_bytes()) {
                Some(row) => editor.entries(&row.oid)?,
                None => vec![],
            };
            files.retain(|r| r.name != name.as_bytes());
            files.push(RawEntry {
                mode: "100644".into(),
                name: name.as_bytes().to_vec(),
                oid: editor.blob(bytes)?,
            });
            let tree = editor.tree(&files)?;
            roots.retain(|r| r.name != folder.as_bytes());
            roots.push(RawEntry {
                mode: "040000".into(),
                name: folder.as_bytes().to_vec(),
                oid: tree,
            });
            editor.tree(&roots)
        })?;
        Ok(())
    }
    pub fn fixture_blob_id(bytes: &[u8]) -> String {
        storage::blob_id(bytes)
    }
    pub fn fixture_refuse_next_write(&self) -> Result<(), BoardStoreError> {
        self.hooks.arm(Fault::Refuse)
    }
    pub fn fixture_fail_after_first_row(&self) -> Result<(), BoardStoreError> {
        self.hooks.arm(Fault::AfterFirstRow)
    }
    pub fn fixture_take_payload_fetches(&self) -> Vec<(String, String, usize)> {
        std::mem::take(
            &mut *self
                .payload_fetches
                .lock()
                .expect("payload observation lock"),
        )
    }
    pub fn connection_settings(&self) -> Result<(String, i64), BoardStoreError> {
        let c = self.connection.lock().map_err(unavailable)?;
        Ok((
            c.query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .map_err(unavailable)?,
            c.query_row("PRAGMA synchronous", [], |r| r.get(0))
                .map_err(unavailable)?,
        ))
    }
    pub fn fixture_committed_by(&self, head: &BoardHead) -> Result<Identity, BoardStoreError> {
        self.connection
            .lock()
            .map_err(unavailable)?
            .query_row(
                "SELECT name,email FROM revisions WHERE head=?1",
                [&head.0],
                |r| {
                    Ok(Identity {
                        name: r.get(0)?,
                        email: r.get(1)?,
                    })
                },
            )
            .map_err(unavailable)
    }
    pub fn fixture_head(&self) -> Result<BoardHead, BoardStoreError> {
        storage::current(&*self.connection.lock().map_err(unavailable)?).map(|(head, _)| head)
    }
    pub fn fixture_object_absent(&self, oid: &str) -> Result<bool, BoardStoreError> {
        self.connection
            .lock()
            .map_err(unavailable)?
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM revisions WHERE head=?1)",
                [oid],
                |r| r.get(0),
            )
            .map_err(unavailable)
    }
    pub fn fixture_object_size(&self, oid: &str) -> Result<usize, BoardStoreError> {
        let c = self.connection.lock().map_err(unavailable)?;
        let size:i64=c.query_row("SELECT CASE kind WHEN 'blob' THEN length(bytes) WHEN 'tree' THEN tree_size END FROM objects WHERE oid=?1",[oid],|r| r.get(0)).map_err(unavailable)?;
        usize::try_from(size).map_err(unavailable)
    }
    pub fn fixture_fingerprint(&self) -> Result<String, BoardStoreError> {
        use rusqlite::types::ValueRef;
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c.transaction().map_err(unavailable)?;
        let mut hash = Sha256::new();
        let mut tables = vec![
            ("objects", "oid"),
            ("tree_entries", "tree,ordinal"),
            ("revisions", "head"),
            ("state", "id"),
        ];
        // The RED baseline is schema2. Schema3 retains all original queries and
        // adds every history table; SELECT * also includes all revision columns.
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(unavailable)?;
        if version == 3 {
            tables.extend([
                ("git_objects", "oid"),
                ("history_meta", "id"),
                ("epochs", "epoch"),
            ]);
        }
        for (table, order) in tables {
            let mut query = tx
                .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
                .map_err(unavailable)?;
            let columns = query.column_count();
            let mut rows = query.query([]).map_err(unavailable)?;
            hash.update((table.len() as u64).to_be_bytes());
            hash.update(table);
            hash.update((columns as u64).to_be_bytes());
            let mut count = 0u64;
            while let Some(row) = rows.next().map_err(unavailable)? {
                hash.update(b"row");
                count += 1;
                for column in 0..columns {
                    let (kind, value): (u8, Vec<u8>) =
                        match row.get_ref(column).map_err(unavailable)? {
                            ValueRef::Null => (0, vec![]),
                            ValueRef::Integer(v) => (1, v.to_be_bytes().to_vec()),
                            ValueRef::Real(v) => (2, v.to_bits().to_be_bytes().to_vec()),
                            ValueRef::Text(v) => (3, v.to_vec()),
                            ValueRef::Blob(v) => (4, v.to_vec()),
                        };
                    hash.update([kind]);
                    hash.update((value.len() as u64).to_be_bytes());
                    hash.update(value);
                }
            }
            hash.update(b"end");
            hash.update(count.to_be_bytes());
        }
        let (_, root) = storage::current(&tx)?;
        let archive:Option<String>=tx.query_row("SELECT mode||' tree '||oid||char(9)||'archive' FROM tree_entries WHERE tree=?1 AND raw_name=CAST('archive' AS BLOB)",[root],|r| r.get(0)).optional().map_err(unavailable)?;
        Ok(format!(
            "{:x}\n{}",
            hash.finalize(),
            archive.unwrap_or_default()
        ))
    }
}

#[cfg(test)]
pub(super) fn poison_for_test(hooks: &std::sync::Arc<FixtureHooks>) {
    let hooks = hooks.clone();
    assert!(
        std::thread::spawn(move || {
            let _guard = hooks.0.lock().unwrap();
            panic!("owned hook poison probe");
        })
        .join()
        .is_err()
    );
}
