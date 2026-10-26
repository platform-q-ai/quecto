//! A follower's board on SQLite (epic #2705 B11): a batch is applied in one
//! transaction under the write lock: each object at its place, each
//! revision after the one before it with its chain made again from the
//! rows and matched to the primary's, then the epochs and provenance; the
//! new revisions go to the follower's own feed. A batch that is not the
//! next one is refused whole, and a fork is never rewritten.
use super::replica_log::number;
use super::*;
use crate::application::project_board::ports::replication::*;

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

    fn apply(&self, batch: &ReplicaBatch) -> Result<ReplicaPosition, ReplicaError> {
        let (mut published, mut c) = self.locked_for_apply().map_err(store_failed)?;
        let applied = (|| {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| store_failed(unavailable(error)))?;
            let at = position_on(&tx).map_err(store_failed)?;
            if (at.revision, &at.chain) != (batch.after, &batch.chain_after) {
                return Err(ReplicaError::Diverged(format!(
                    "the batch follows revision {}, but this board is at revision {} \
                     (or its chain differs)",
                    batch.after, at.revision
                )));
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
                put_meta(&mut writer, meta, self.bound())?;
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
    match &object.content {
        ReplicaContent::Blob(bytes) => {
            // A native object's id is its content's: one made here must match.
            match (&object.original, storage::blob_id(bytes) == object.oid) {
                (Some(None), _) | (None, true) => {}
                (Some(Some(_)) | None, _) => {
                    return Err(invalid(format!(
                        "blob {} is not as its id says",
                        object.oid
                    )));
                }
            }
            writer
                .insert(
                    "INSERT INTO objects (rowid,oid,kind,bytes,tree_size) VALUES (?1,?2,'blob',?3,NULL)",
                    params![place, object.oid, bytes],
                )
                .map_err(store_failed)?;
        }
        ReplicaContent::Tree { size, entries } => {
            let rows: Vec<_> = entries
                .iter()
                .map(|entry| storage::entry(&entry.mode, &entry.raw_name, entry.oid.clone()))
                .collect();
            let (native, native_size) = storage::tree_digest(&rows).map_err(store_failed)?;
            let size = i64::try_from(*size).map_err(|error| invalid(error.to_string()))?;
            match (
                &object.original,
                native == object.oid && native_size == size,
            ) {
                (Some(Some(_)), _) | (None, true) => {}
                (Some(None) | None, _) => {
                    return Err(invalid(format!(
                        "tree {} is not as its id says",
                        object.oid
                    )));
                }
            }
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

/// Holds `epochs`: each new one added, each held one the same.
fn put_epochs(
    c: &Connection,
    writer: &mut storage::ObjectWriter<'_>,
    epochs: &[ReplicaEpoch],
) -> Result<(), ReplicaError> {
    for epoch in epochs {
        let (number, from) = (epoch.epoch as i64, epoch.from_revision as i64);
        let row = params![number, from, epoch.reason, epoch.at];
        writer
            .insert(
                "INSERT INTO epochs VALUES (?1,?2,?3,?4) ON CONFLICT(epoch) DO NOTHING",
                row,
            )
            .map_err(store_failed)?;
        let same: bool = c
            .query_row(
                "SELECT count(*)=1 FROM epochs WHERE epoch=?1 AND from_revision=?2 AND reason=?3 AND at=?4",
                params![number, from, epoch.reason, epoch.at],
                |r| r.get(0),
            )
            .map_err(|error| store_failed(unavailable(error)))?;
        if !same {
            return Err(ReplicaError::Diverged(format!(
                "epoch {} differs from the primary's",
                epoch.epoch
            )));
        }
    }
    Ok(())
}

/// Holds the primary's provenance row, which must name this board.
fn put_meta(
    writer: &mut storage::ObjectWriter<'_>,
    meta: &ReplicaMeta,
    bound: Option<&str>,
) -> Result<(), ReplicaError> {
    match (meta.board_id.as_deref(), bound) {
        (Some(primary), Some(here)) if primary != here => {
            return Err(ReplicaError::Invalid(format!(
                "the primary serves board {primary}, not this server's board {here}"
            )));
        }
        (_, _) => {}
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
