//! A primary's history for its followers on the SQLite board (epic #2705
//! B11), read in one read transaction: its objects in their order (each
//! row's place, 1, 2, .. with no gap: objects are never deleted), then its
//! revisions only once every object it holds is sent, so each revision's
//! objects come before it; with its chain at the follower's revision, its
//! epochs and its provenance. A batch holds about [`BATCH_BYTES`] of rows,
//! and one at least.
use super::*;
use crate::application::project_board::ports::replication::*;

/// What a batch row costs beside its bytes.
const ROW_BYTES: usize = 128;

pub(super) fn number(value: i64) -> Result<u64, BoardStoreError> {
    u64::try_from(value).map_err(unavailable)
}

impl ReplicationLog for SqliteBoardOps {
    fn replica_batch(
        &self,
        after: u64,
        objects_after: u64,
        limit: BatchLimit,
    ) -> Result<ReplicaBatch, BoardStoreError> {
        let mut c = self.reads.connection.lock().map_err(unavailable)?;
        let tx = storage::Scope::begin(&mut c, false)?;
        let (after_sql, objects_sql) = (
            i64::try_from(after).unwrap_or(i64::MAX),
            i64::try_from(objects_after).unwrap_or(i64::MAX),
        );
        let tip = feed::tip(&tx)?;
        let chain_after: Option<String> = tx
            .query_row(
                "SELECT chain FROM revisions WHERE revision=?1",
                [after_sql],
                |r| r.get(0),
            )
            .optional()
            .map_err(unavailable)?
            .flatten();
        let held: i64 = tx
            .query_row("SELECT coalesce(max(rowid),0) FROM objects", [], |r| {
                r.get(0)
            })
            .map_err(unavailable)?;
        let mut spent = 0usize;
        let objects = objects_from(&tx, objects_sql, &mut spent)?;
        let sent = objects
            .last()
            .map_or(objects_sql, |last| last.sequence as i64);
        let revisions = match sent >= held {
            true => revisions_from(&tx, after_sql, limit, &mut spent)?,
            false => Vec::new(),
        };
        let last = revisions.last().map_or(after, |last| last.revision);
        Ok(ReplicaBatch {
            after,
            chain_after,
            objects,
            more: sent < held || last < tip.revision,
            revisions,
            epochs: epochs(&tx)?,
            meta: meta(&tx)?,
            tokens: Vec::new(),
            tip: tip.revision,
        })
    }
}

/// The objects after place `after`, in order, until about [`BATCH_BYTES`]
/// are held (one at least).
fn objects_from(
    c: &Connection,
    after: i64,
    spent: &mut usize,
) -> Result<Vec<ReplicaObject>, BoardStoreError> {
    type Row = (
        i64,
        String,
        String,
        Option<Vec<u8>>,
        Option<i64>,
        bool,
        Option<Vec<u8>>,
    );
    let mut statement = c
        .prepare(
            "SELECT o.rowid,o.oid,o.kind,o.bytes,o.tree_size,g.oid IS NOT NULL,g.raw_tree \
             FROM objects o LEFT JOIN git_objects g ON g.oid=o.oid WHERE o.rowid>?1 ORDER BY o.rowid",
        )
        .map_err(unavailable)?;
    let mut rows = statement.query([after]).map_err(unavailable)?;
    let mut objects = Vec::new();
    while *spent < BATCH_BYTES || objects.is_empty() {
        let Some(row) = rows.next().map_err(unavailable)? else {
            break;
        };
        let read = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        };
        let (sequence, oid, kind, bytes, size, original, raw) = read(row).map_err(unavailable)?;
        let content = match (kind.as_str(), bytes, size) {
            ("blob", Some(bytes), None) => ReplicaContent::Blob(bytes),
            ("tree", None, Some(size)) => ReplicaContent::Tree {
                size: number(size)?,
                entries: entries(c, &oid)?,
            },
            _ => return Err(unavailable(format!("object {oid} is malformed"))),
        };
        *spent += ROW_BYTES
            + raw.as_ref().map_or(0, Vec::len)
            + match &content {
                ReplicaContent::Blob(bytes) => bytes.len(),
                ReplicaContent::Tree { entries, .. } => entries
                    .iter()
                    .map(|entry| ROW_BYTES + entry.raw_name.len())
                    .sum(),
            };
        objects.push(ReplicaObject {
            sequence: number(sequence)?,
            oid,
            content,
            original: original.then_some(raw),
        });
    }
    Ok(objects)
}

fn entries(c: &Connection, tree: &str) -> Result<Vec<ReplicaEntry>, BoardStoreError> {
    let mut statement = c
        .prepare_cached("SELECT mode,raw_name,oid FROM tree_entries WHERE tree=?1 ORDER BY ordinal")
        .map_err(unavailable)?;
    let rows = statement
        .query_map([tree], |r| {
            Ok(ReplicaEntry {
                mode: r.get(0)?,
                raw_name: r.get(1)?,
                oid: r.get(2)?,
            })
        })
        .map_err(unavailable)?;
    rows.collect::<Result<_, _>>().map_err(unavailable)
}

/// The revisions after `after`, at most `limit`, until about
/// [`BATCH_BYTES`] are held (one at least).
fn revisions_from(
    c: &Connection,
    after: i64,
    limit: BatchLimit,
    spent: &mut usize,
) -> Result<Vec<ReplicaRevision>, BoardStoreError> {
    let mut statement = c
        .prepare(
            "SELECT r.head,r.root,r.name,r.email,r.revision,r.parent,r.message,r.at,r.git_commit,\
             r.git_meta,p.principal,r.chain FROM revisions r LEFT JOIN revision_principals p \
             ON p.head=r.head WHERE r.revision>?1 ORDER BY r.revision LIMIT ?2",
        )
        .map_err(unavailable)?;
    let limit = i64::try_from(limit.get()).map_err(unavailable)?;
    let mut rows = statement.query([after, limit]).map_err(unavailable)?;
    let mut revisions = Vec::new();
    while *spent < BATCH_BYTES || revisions.is_empty() {
        let Some(r) = rows.next().map_err(unavailable)? else {
            break;
        };
        let parent: Option<i64> = r.get(5).map_err(unavailable)?;
        let chain: Option<String> = r.get(11).map_err(unavailable)?;
        let revision = ReplicaRevision {
            head: r.get(0).map_err(unavailable)?,
            root: r.get(1).map_err(unavailable)?,
            name: r.get(2).map_err(unavailable)?,
            email: r.get(3).map_err(unavailable)?,
            revision: number(r.get(4).map_err(unavailable)?)?,
            parent: parent.map(number).transpose()?,
            message: r.get(6).map_err(unavailable)?,
            at: r.get(7).map_err(unavailable)?,
            git_commit: r.get(8).map_err(unavailable)?,
            git_meta: r.get(9).map_err(unavailable)?,
            principal: r.get(10).map_err(unavailable)?,
            chain: chain.ok_or_else(|| unavailable("a revision has no chain"))?,
        };
        *spent +=
            ROW_BYTES + revision.message.len() + revision.git_meta.as_ref().map_or(0, Vec::len);
        revisions.push(revision);
    }
    Ok(revisions)
}

fn epochs(c: &Connection) -> Result<Vec<ReplicaEpoch>, BoardStoreError> {
    let mut statement = c
        .prepare("SELECT epoch,from_revision,reason,at FROM epochs ORDER BY epoch")
        .map_err(unavailable)?;
    let rows = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(unavailable)?;
    rows.map(|row| {
        let (epoch, from, reason, at): (i64, i64, String, i64) = row.map_err(unavailable)?;
        Ok(ReplicaEpoch {
            epoch: number(epoch)?,
            from_revision: number(from)?,
            reason,
            at,
        })
    })
    .collect()
}

fn meta(c: &Connection) -> Result<Option<ReplicaMeta>, BoardStoreError> {
    type Row = (
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let row: Option<Row> = c
        .query_row(
            "SELECT board_id,algorithm,source_remote,source_ref,source_head,imported_count \
             FROM history_meta WHERE id=1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(unavailable)?;
    row.map(
        |(board_id, algorithm, source_remote, source_ref, source_head, count)| {
            Ok(ReplicaMeta {
                board_id,
                algorithm,
                source_remote,
                source_ref,
                source_head,
                imported_count: number(count)?,
            })
        },
    )
    .transpose()
}
