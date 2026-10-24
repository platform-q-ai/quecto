//! A transaction-borrowing adapter to the one shared board interpreter.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{
    Entry, Found, LeftOnRemote, Object, Reader, Unread, raw_entry_bytes, read_root,
};

pub(super) struct SqlReader<'a> {
    pub connection: &'a Connection,
    #[cfg(feature = "test-support")]
    pub fetches: &'a Mutex<Vec<(String, String, usize)>>,
}
impl Reader for SqlReader<'_> {
    fn metadata(&self, ids: &[&str]) -> Result<BTreeMap<String, Found>, BoardStoreError> {
        let mut found = BTreeMap::new();
        let mut query = self.connection.prepare("SELECT kind,CASE kind WHEN 'blob' THEN length(bytes) WHEN 'tree' THEN tree_size END FROM objects WHERE oid=?1").map_err(unavailable)?;
        for id in ids {
            let row: Option<(String, i64)> = query
                .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .map_err(unavailable)?;
            let object = match row {
                Some((kind, size)) if matches!(kind.as_str(), "blob" | "tree") => {
                    Found::Present(Object {
                        kind,
                        size: usize::try_from(size).map_err(unavailable)?,
                        omitted: false,
                    })
                }
                Some(_) => return Err(unavailable("unsupported stored object kind")),
                None => Found::Missing,
            };
            found.insert((*id).into(), object);
        }
        Ok(found)
    }
    fn listing(&self, tree: &str, cap: usize) -> Result<Vec<Entry>, Unread> {
        let c = self.connection;
        // Aggregate lengths before any raw name is allocated. Stored tree_size
        // alone is untrusted and cannot authorize an unbounded listing.
        let (count, allocation, valid): (i64,i64,bool) = c.query_row("SELECT count(*),coalesce(sum(length(raw_name)*4+length(mode)+length(oid)+128),0),coalesce(min(typeof(raw_name)='blob' AND length(mode) BETWEEN 1 AND 6 AND length(oid) IN (40,64)),1) FROM tree_entries WHERE tree=?1", [tree], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(unavailable)?;
        let count = usize::try_from(count).map_err(unavailable)?;
        let allocation = usize::try_from(allocation).map_err(unavailable)?;
        match valid && count <= cap && allocation <= cap {
            true => {}
            false => {
                return Err(unavailable("stored tree listing exceeds allocation bounds").into());
            }
        }
        let stored: i64 = c
            .query_row(
                "SELECT tree_size FROM objects WHERE oid=?1 AND kind='tree'",
                [tree],
                |r| r.get(0),
            )
            .map_err(unavailable)?;
        let mut query = c
            .prepare(
                "SELECT ordinal,mode,raw_name,oid FROM tree_entries WHERE tree=?1 ORDER BY ordinal",
            )
            .map_err(unavailable)?;
        let mut rows = query.query([tree]).map_err(unavailable)?;
        let mut entries = Vec::with_capacity(count);
        let mut size = 0usize;
        while let Some(row) = rows.next().map_err(unavailable)? {
            let ordinal: i64 = row.get(0).map_err(unavailable)?;
            let mode: String = row.get(1).map_err(unavailable)?;
            let raw: Vec<u8> = row.get(2).map_err(unavailable)?;
            let oid: String = row.get(3).map_err(unavailable)?;
            match ordinal == entries.len() as i64 && object_id(&oid).is_ok() {
                true => {}
                false => return Err(unavailable("invalid stored tree entry").into()),
            }
            size = size
                .checked_add(raw_entry_bytes(&mode, &raw, &oid))
                .ok_or_else(|| unavailable("tree size overflow"))?;
            entries.push(storage::entry(&mode, &raw, oid));
        }
        match size == usize::try_from(stored).map_err(unavailable)? && entries.len() == count {
            true => Ok(entries),
            false => Err(unavailable("stored tree size or cardinality mismatch").into()),
        }
    }
    fn admitted(
        &self,
        wanted: &[(&str, usize)],
    ) -> Result<BTreeMap<String, Vec<u8>>, BoardStoreError> {
        let mut query = self
            .connection
            .prepare("SELECT bytes FROM objects WHERE oid=?1 AND kind='blob' AND length(bytes)=?2")
            .map_err(unavailable)?;
        let mut content = BTreeMap::new();
        for (id, size) in wanted {
            // One immutable object may occur under thousands of names. The
            // shared content contract is identity-keyed, so fetch it once.
            if content.contains_key(*id) {
                continue;
            }
            let bytes: Vec<u8> = query
                .query_row(
                    params![id, i64::try_from(*size).map_err(unavailable)?],
                    |r| r.get(0),
                )
                .map_err(unavailable)?;
            #[cfg(feature = "test-support")]
            {
                // Observe actual admitted artifact BLOB materialization. FORMAT
                // is independently bounded by the shared reader, not an artifact.
                let mut paths = self.connection.prepare("SELECT CAST(root.raw_name AS TEXT)||'/'||CAST(file.raw_name AS TEXT) FROM state JOIN revisions ON revisions.head=state.head JOIN tree_entries root ON root.tree=revisions.root JOIN tree_entries file ON file.tree=root.oid WHERE root.raw_name IN (CAST('tasks' AS BLOB),CAST('plans' AS BLOB),CAST('runs' AS BLOB),CAST('changes' AS BLOB),CAST('reviews' AS BLOB),CAST('discussions' AS BLOB)) AND file.oid=?1 ORDER BY root.ordinal,file.ordinal").map_err(unavailable)?;
                for path in paths
                    .query_map([id], |r| r.get::<_, String>(0))
                    .map_err(unavailable)?
                {
                    self.fetches.lock().map_err(unavailable)?.push((
                        path.map_err(unavailable)?,
                        (*id).into(),
                        bytes.len(),
                    ));
                }
            }
            content.insert((*id).into(), bytes);
        }
        Ok(content)
    }
}
impl SqliteProjectBoardStore {
    pub(super) fn reader<'a>(&'a self, c: &'a Connection) -> SqlReader<'a> {
        SqlReader {
            connection: c,
            #[cfg(feature = "test-support")]
            fetches: &self.payload_fetches,
        }
    }
    pub(super) fn board(&self, c: &Connection, root: &str) -> Result<Board, BoardStoreError> {
        read_root(&self.reader(c), root, LIMITS, &LeftOnRemote::default()).map_err(|error| {
            match error {
                Unread::Failed(error) => error,
                Unread::Missing(_) => unavailable("missing SQLite board object"),
            }
        })
    }
}
