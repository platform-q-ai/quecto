//! A follower's board on SQLite (epic #2705 B11): a batch is applied in one
//! transaction under the write lock: each object at its place, each
//! revision after the one before it with its chain made again from the
//! rows and matched to the primary's, then the epochs and provenance; the
//! new revisions go to the follower's own feed. A batch that is not the
//! next one is refused whole, and a fork is never rewritten.
use super::replica_log::number;
use super::*;
use crate::application::project_board::ports::replication::*;
use crate::infrastructure::persistence::file::token_file::{read_private, write_private};

/// A follower's place, read on `c`.
fn position_on(c: &Connection) -> Result<ReplicaPosition, BoardStoreError> {
    let (revision, chain, objects): (i64, Option<String>, i64) = c
        .query_row(
            "SELECT coalesce(max(revision),0), \
             (SELECT chain FROM revisions WHERE revision=(SELECT max(revision) FROM revisions)), \
             (SELECT coalesce(max(rowid),0) FROM objects) FROM revisions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(unavailable)?;
    Ok(ReplicaPosition {
        revision: number(revision)?,
        chain,
        objects: number(objects)?,
    })
}

/// The most bytes a diverged mark holds.
/// The diverged mark beside the board file at `path`.
fn mark_of(path: &Path) -> std::path::PathBuf {
    let mut mark = path.as_os_str().to_owned();
    mark.push(".diverged");
    std::path::PathBuf::from(mark)
}

/// What the diverged mark is called in its errors.
const MARK: &str = "the diverged mark";

fn store_failed(error: BoardStoreError) -> ReplicaError {
    ReplicaError::Unavailable(format!("{error:?}"))
}

impl ReplicaStore for SqliteBoardOps {
    fn position(&self) -> Result<ReplicaPosition, ReplicaError> {
        let mut c = self
            .reads
            .connection
            .lock()
            .map_err(unavailable)
            .map_err(store_failed)?;
        let tx = storage::Scope::begin(&mut c, false).map_err(store_failed)?;
        position_on(&tx).map_err(store_failed)
    }

    fn diverged(&self) -> Result<Option<String>, ReplicaError> {
        let held = read_private(&mark_of(&self.path), MARK);
        let held = held.map_err(ReplicaError::Unavailable)?;
        Ok(held.map(|why| why.trim_end().to_string()))
    }

    fn mark_diverged(&self, why: &str) -> Result<(), ReplicaError> {
        let why: String = why.chars().take(1024).collect();
        write_private(&mark_of(&self.path), MARK, &format!("{why}\n"))
            .map_err(ReplicaError::Unavailable)
    }

    fn apply(&self, batch: &ReplicaBatch) -> Result<ReplicaPosition, ReplicaError> {
        let (mut published, mut c) = self.locked_for_apply().map_err(store_failed)?;
        let applied = (|| {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| store_failed(unavailable(error)))?;
            let at = position_on(&tx).map_err(store_failed)?;
            match (at.revision == batch.after, at.chain == batch.chain_after) {
                (true, true) => {}
                (_, _) => {
                    return Err(ReplicaError::Diverged(format!(
                        "the batch follows revision {}, but this board is at revision {} \
                         (or its chain differs)",
                        batch.after, at.revision
                    )));
                }
            }
            let mut writer = storage::ObjectWriter::new(&tx);
            let mut next = at.objects;
            for object in &batch.objects {
                next += 1;
                put_object(&mut writer, object, next)?;
            }
            let mut last = at.revision;
            for revision in &batch.revisions {
                put_revision(&tx, &mut writer, revision, last)?;
                last = revision.revision;
            }
            put_epochs(&tx, &mut writer, &batch.epochs)?;
            if let Some(meta) = &batch.meta {
                put_meta(&tx, &mut writer, meta, self.bound())?;
            }
            if let Some(head) = batch.revisions.last().map(|revision| &revision.head) {
                writer
                    .insert(
                        "INSERT INTO state VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET head=excluded.head",
                        [head],
                    )
                    .map_err(store_failed)?;
            }
            let now = position_on(&tx).map_err(store_failed)?;
            tx.commit()
                .map_err(|error| store_failed(unavailable(error)))?;
            Ok(now)
        })()?;
        self.publish(&c, &mut published);
        Ok(applied)
    }
}

/// Puts `object` at place `place`, which must be its primary's.
fn put_object(
    writer: &mut storage::ObjectWriter<'_>,
    object: &ReplicaObject,
    place: u64,
) -> Result<(), ReplicaError> {
    let invalid = |why: String| ReplicaError::Invalid(why);
    if object.sequence != place {
        return Err(invalid(format!(
            "object {} is at place {} on the primary, not {place}: a batch's objects come in order",
            object.oid, object.sequence
        )));
    }
    let place = i64::try_from(place).map_err(|error| invalid(error.to_string()))?;
    let kind = match object.content {
        ReplicaContent::Blob(_) => "blob",
        ReplicaContent::Tree { .. } => "tree",
    };
    match as_its_id_says(object) {
        true => {}
        false => {
            return Err(invalid(format!(
                "{kind} {} is not as its id says",
                object.oid
            )));
        }
    }
    match &object.content {
        ReplicaContent::Blob(bytes) => {
            writer
                .insert(
                    "INSERT INTO objects (rowid,oid,kind,bytes,tree_size) VALUES (?1,?2,'blob',?3,NULL)",
                    params![place, object.oid, bytes],
                )
                .map_err(store_failed)?;
        }
        ReplicaContent::Tree { size, entries } => {
            let size = i64::try_from(*size).map_err(|error| invalid(error.to_string()))?;
            writer
                .insert(
                    "INSERT INTO objects (rowid,oid,kind,bytes,tree_size) VALUES (?1,?2,'tree',NULL,?3)",
                    params![place, object.oid, size],
                )
                .map_err(store_failed)?;
            for (ordinal, entry) in entries.iter().enumerate() {
                writer
                    .insert(
                        "INSERT INTO tree_entries VALUES (?1,?2,?3,?4,?5)",
                        params![
                            object.oid,
                            ordinal as i64,
                            entry.mode,
                            entry.raw_name,
                            entry.oid
                        ],
                    )
                    .map_err(store_failed)?;
            }
        }
    }
    if let Some(raw) = &object.original {
        writer
            .insert(
                "INSERT INTO git_objects VALUES (?1,?2)",
                params![object.oid, raw],
            )
            .map_err(store_failed)?;
    }
    Ok(())
}

/// Whether `object` is as its id says, an allowlist: a native object's id
/// made again here from its content; an imported original's git id made
/// from its raw bytes by its board's algorithm (an id of 40 hex digits is
/// SHA-1's, of 64 SHA-256's), a tree's entries read from its raw tree and
/// its size theirs.
fn as_its_id_says(object: &ReplicaObject) -> bool {
    use crate::application::project_board::ports::history::{
        HistoryEntry, HistoryKind, HistoryObject,
    };
    use crate::infrastructure::persistence::project_board::board_tree::raw_entry_bytes;
    use crate::infrastructure::persistence::project_board::history::verify_raw;
    let width = object.oid.len();
    let git = [40, 64].contains(&width);
    let original = |kind, bytes: &[u8], entries: &[ReplicaEntry]| HistoryObject {
        oid: object.oid.clone(),
        kind,
        bytes: bytes.to_vec(),
        entries: entries
            .iter()
            .map(|entry| HistoryEntry {
                mode: entry.mode.clone(),
                name: entry.raw_name.clone(),
                oid: entry.oid.clone(),
            })
            .collect(),
    };
    match (&object.content, &object.original) {
        (ReplicaContent::Blob(bytes), None) => storage::blob_id(bytes) == object.oid,
        (ReplicaContent::Tree { size, entries }, None) => {
            let rows: Vec<_> = entries
                .iter()
                .map(|entry| storage::entry(&entry.mode, &entry.raw_name, entry.oid.clone()))
                .collect();
            let native = storage::tree_digest(&rows).ok();
            native.is_some_and(|(oid, made)| oid == object.oid && u64::try_from(made) == Ok(*size))
        }
        (ReplicaContent::Blob(bytes), Some(None)) if git => {
            verify_raw(&original(HistoryKind::Blob, bytes, &[]), width).is_ok()
        }
        (ReplicaContent::Tree { size, entries }, Some(Some(raw))) if git => {
            let tree = original(HistoryKind::Tree, raw, entries);
            let made = entries.iter().try_fold(0usize, |sum, entry| {
                sum.checked_add(raw_entry_bytes(&entry.mode, &entry.raw_name, &entry.oid))
            });
            verify_raw(&tree, width).is_ok()
                && super::history::projection(&tree, width).is_ok()
                && made.and_then(|made| u64::try_from(made).ok()) == Some(*size)
        }
        (_, _) => false,
    }
}

/// Puts `revision` after revision `last`, its chain made again here.
fn put_revision(
    c: &Connection,
    writer: &mut storage::ObjectWriter<'_>,
    revision: &ReplicaRevision,
    last: u64,
) -> Result<(), ReplicaError> {
    let invalid = |why: String| ReplicaError::Invalid(why);
    let number = revision.revision;
    let parent_ok = match number {
        1 => revision.parent.is_none(),
        _ => revision.parent == Some(number - 1),
    };
    let rooted: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE oid=?1 AND kind='tree')",
            [&revision.root],
            |r| r.get(0),
        )
        .map_err(|error| store_failed(unavailable(error)))?;
    match (Some(number) == last.checked_add(1), parent_ok, rooted) {
        (true, true, true) => {}
        _ => {
            return Err(invalid(format!(
                "revision {number} does not follow revision {last} on a tree this board holds"
            )));
        }
    }
    let sql = |value: u64| i64::try_from(value).map_err(|error| invalid(error.to_string()));
    let at = sql(number)?;
    writer
        .insert(
            "INSERT INTO revisions VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL)",
            params![
                revision.head,
                revision.root,
                revision.name,
                revision.email,
                at,
                revision.parent.map(sql).transpose()?,
                revision.message,
                revision.at,
                revision.git_commit,
                revision.git_meta
            ],
        )
        .map_err(store_failed)?;
    if let Some(principal) = &revision.principal {
        writer
            .insert(
                "INSERT INTO revision_principals VALUES (?1,?2)",
                params![revision.head, principal],
            )
            .map_err(store_failed)?;
    }
    let chain = writer.seal(at).map_err(store_failed)?;
    match chain == revision.chain {
        true => Ok(()),
        false => Err(invalid(format!(
            "revision {number}'s rows do not make the chain the primary sent"
        ))),
    }
}

/// Holds `epochs`, an allowlist: an epoch held is the primary's row as it
/// is held; one not held is the next after those held (1 with none). An
/// epoch held otherwise is a fork; one past a gap is refused.
fn put_epochs(
    c: &Connection,
    writer: &mut storage::ObjectWriter<'_>,
    epochs: &[ReplicaEpoch],
) -> Result<(), ReplicaError> {
    let read = |error: &dyn std::fmt::Display| store_failed(unavailable(error));
    for epoch in epochs {
        let number = i64::try_from(epoch.epoch).map_err(|e| read(&e))?;
        let from = i64::try_from(epoch.from_revision).map_err(|e| read(&e))?;
        let held: Option<(i64, String, i64)> = c
            .query_row(
                "SELECT from_revision,reason,at FROM epochs WHERE epoch=?1",
                [number],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| read(&e))?;
        let last: i64 = c
            .query_row("SELECT coalesce(max(epoch),0) FROM epochs", [], |r| {
                r.get(0)
            })
            .map_err(|e| read(&e))?;
        match held {
            Some(row) if row == (from, epoch.reason.clone(), epoch.at) => {}
            Some(_) => {
                return Err(ReplicaError::Diverged(format!(
                    "epoch {} differs from the primary's",
                    epoch.epoch
                )));
            }
            None if Some(number) == last.checked_add(1) => {
                let row = params![number, from, epoch.reason, epoch.at];
                (writer.insert("INSERT INTO epochs VALUES (?1,?2,?3,?4)", row))
                    .map_err(store_failed)?;
            }
            None => {
                return Err(ReplicaError::Invalid(format!(
                    "epoch {} does not follow epoch {last}",
                    epoch.epoch
                )));
            }
        }
    }
    Ok(())
}

/// Holds the primary's provenance row, an allowlist: it names this
/// server's board, or it names a board and this server is bound to none.
/// A binding is never overwritten, and never with none.
fn put_meta(
    c: &Connection,
    writer: &mut storage::ObjectWriter<'_>,
    meta: &ReplicaMeta,
    bound: Option<&str>,
) -> Result<(), ReplicaError> {
    match (meta.board_id.as_deref(), bound) {
        (Some(primary), Some(here)) if primary == here => {}
        (Some(_), None) => {}
        (Some(primary), Some(here)) => {
            return Err(ReplicaError::Invalid(format!(
                "the primary serves board {primary}, not this server's board {here}"
            )));
        }
        (None, _) => {
            return Err(ReplicaError::Invalid(
                "the primary's board names no board id: it is not copied".into(),
            ));
        }
    }
    // The provenance held, an allowlist: none yet; the primary's, field
    // for field; or the primary's but for a board id this unbound board
    // has none of. Any other is another board's history.
    let held = super::replica_log::meta(c).map_err(store_failed)?;
    let unbound = |held: &ReplicaMeta| {
        let bound = ReplicaMeta {
            board_id: meta.board_id.clone(),
            ..held.clone()
        };
        held.board_id.is_none() && bound == *meta
    };
    match held {
        None => {}
        Some(held) if held == *meta => return Ok(()),
        Some(held) if unbound(&held) => {}
        Some(_) => {
            return Err(ReplicaError::Diverged(
                "the primary's provenance (its algorithm or import source) is not this board's"
                    .into(),
            ));
        }
    }
    let count = i64::try_from(meta.imported_count)
        .map_err(|error| ReplicaError::Invalid(error.to_string()))?;
    writer
        .insert(
            "INSERT INTO history_meta VALUES (1,?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET \
             board_id=excluded.board_id, algorithm=excluded.algorithm, \
             source_remote=excluded.source_remote, source_ref=excluded.source_ref, \
             source_head=excluded.source_head, imported_count=excluded.imported_count",
            params![
                meta.board_id,
                meta.algorithm,
                meta.source_remote,
                meta.source_ref,
                meta.source_head,
                count
            ],
        )
        .map_err(store_failed)?;
    Ok(())
}

#[cfg(test)]
#[path = "replica_store_tests.rs"]
mod tests;
