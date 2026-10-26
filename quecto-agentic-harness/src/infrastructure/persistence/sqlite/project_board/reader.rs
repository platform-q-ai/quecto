//! A transaction-borrowing adapter to the one shared board interpreter.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{
    Entry, Found, LeftOnRemote, Object, Reader, Unread, raw_entry_bytes, read_root,
};

#[cfg(feature = "test-support")]
type Fetches = Mutex<Vec<(String, String, usize)>>;

pub(super) struct SqlReader<'a> {
    pub connection: &'a Connection,
    #[cfg(feature = "test-support")]
    pub fetches: Option<&'a Fetches>,
    #[cfg(feature = "test-support")]
    witnesses: std::cell::RefCell<BTreeMap<String, String>>,
}
impl Reader for SqlReader<'_> {
    #[cfg(feature = "test-support")]
    fn witness_artifacts(&self, paths: &[(String, String)]) {
        let mut witnesses = self.witnesses.borrow_mut();
        witnesses.clear();
        for (path, id) in paths {
            // One witness for one identity-deduplicated physical fetch.
            witnesses.entry(id.clone()).or_insert_with(|| path.clone());
        }
    }
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
                // Names are supplied by the shared interpreter, never decoded
                // from raw aliases. Record only after the actual BLOB fetch.
                if let (Some(path), Some(fetches)) =
                    (self.witnesses.borrow().get(*id), self.fetches)
                {
                    fetches.lock().map_err(unavailable)?.push((
                        path.clone(),
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
            fetches: Some(&self.payload_fetches),
            #[cfg(feature = "test-support")]
            witnesses: std::cell::RefCell::default(),
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

/// The object `tree` lists under `name`, when it lists it once and it is
/// an object of `kind` (`blob` or `tree`); none otherwise (#2705 B9): a
/// name listed twice, or of another kind, a read would set aside. It
/// reads `tree`'s rows by the primary key's `tree` prefix and filters them
/// by name: the schema is fixed (a new index would leave every board's
/// schema refused as unsupported), so no index finds a name directly.
pub(super) fn entry_of(
    c: &Connection,
    tree: &str,
    name: &str,
    kind: &str,
) -> Result<Option<String>, BoardStoreError> {
    let mut query = c
        .prepare(
            "SELECT e.oid, o.kind FROM tree_entries e LEFT JOIN objects o ON o.oid=e.oid \
             WHERE e.tree=?1 AND e.raw_name=?2 LIMIT 2",
        )
        .map_err(unavailable)?;
    let rows: Vec<(String, Option<String>)> = query
        .query_map(params![tree, name.as_bytes()], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(unavailable)?
        .collect::<Result<_, _>>()
        .map_err(unavailable)?;
    match rows.as_slice() {
        [(oid, Some(found))] if found == kind => Ok(Some(object_id(oid)?)),
        _ => Ok(None),
    }
}

/// The same structural reader, without ordinary-read payload observations.
pub(super) fn structural(c: &Connection) -> SqlReader<'_> {
    SqlReader {
        connection: c,
        #[cfg(feature = "test-support")]
        fetches: None,
        #[cfg(feature = "test-support")]
        witnesses: std::cell::RefCell::default(),
    }
}
