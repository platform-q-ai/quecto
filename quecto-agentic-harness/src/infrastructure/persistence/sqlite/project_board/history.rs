//! Native epoch fencing and snapshot-only history adapter.
use super::*;
use crate::application::project_board::ports::history::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use std::path::PathBuf;

pub struct SqliteBoardHistoryStore {
    path: PathBuf,
    writer: Identity,
    board: BoardId,
}
impl SqliteBoardHistoryStore {
    pub fn new(path: PathBuf, writer: Identity, board: BoardId) -> Self {
        Self {
            path,
            writer,
            board,
        }
    }
}
impl SqliteBoardHistoryStore {
    pub fn open_epoch(&self, expected: u64, reason: &str, at: i64) -> HistoryResult<BoardHead> {
        line("epoch reason", reason, 200).map_err(error)?;
        if at >= 0 {
        } else {
            return Err(error("epoch time"));
        }
        let mut c = storage::open(&self.path).map_err(error)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        let (head, _) = storage::current(&tx).map_err(error)?;
        let (epoch, revision) = parse_head(&head.0).map_err(error)?;
        if u64::try_from(epoch).map_err(error)? == expected {
        } else {
            return Err(conflict("epoch changed"));
        }
        let next = epoch
            .checked_add(1)
            .ok_or_else(|| error("epoch overflow"))?;
        storage::ObjectWriter::new(&tx)
            .insert(
                "INSERT INTO epochs VALUES (?1,?2,?3,?4)",
                params![next, revision, reason, at],
            )
            .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(BoardHead(format!("{next}:{revision}")))
    }
}
impl BoardHistoryRead for SqliteBoardHistoryStore {
    fn snapshot(&self) -> HistoryResult<Box<dyn BoardHistorySnapshot>> {
        self.writer.validate("history writer").map_err(error)?;
        let c = Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| HistoryError::Unavailable(e.to_string()))?;
        c.busy_timeout(Duration::from_secs(5)).map_err(error)?;
        c.execute_batch("BEGIN").map_err(error)?;
        let mut view = View {
            c,
            frontier: BoardHead(String::new()),
            tip: 0,
        };
        storage::recognize_on(&view.c, false)
            .map_err(|e| HistoryError::Unsupported(format!("{e:?}")))?;
        validate_native(&view.c, &self.board)?;
        let (frontier, root) = storage::current(&view.c).map_err(error)?;
        let (_, tip) = parse_head(&frontier.0).map_err(error)?;
        use crate::infrastructure::persistence::project_board::board_tree::{
            LeftOnRemote, read_root,
        };
        read_root(
            &reader::structural(&view.c),
            &root,
            LIMITS,
            &LeftOnRemote::default(),
        )
        .map_err(error)?;
        view.frontier = frontier;
        view.tip = u64::try_from(tip).map_err(error)?;
        Ok(Box::new(view))
    }
}

struct View {
    c: Connection,
    frontier: BoardHead,
    tip: u64,
}
impl Drop for View {
    fn drop(&mut self) {
        let _ = self.c.execute_batch("ROLLBACK");
    }
}
fn validate_native(c: &Connection, board: &BoardId) -> HistoryResult<()> {
    type Meta = (
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let (binding,algorithm,remote,reference,source,count): Meta = c.query_row(
        "SELECT board_id,algorithm,source_remote,source_ref,source_head,imported_count FROM history_meta WHERE id=1 AND length(CAST(algorithm AS BLOB))<=6 AND (board_id IS NULL OR length(CAST(board_id AS BLOB))=16) AND (source_remote IS NULL OR length(CAST(source_remote AS BLOB)) BETWEEN 1 AND 200) AND (source_ref IS NULL OR length(CAST(source_ref AS BLOB))<=64) AND (source_head IS NULL OR length(CAST(source_head AS BLOB))<=64)", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(error)?;
    if binding.as_deref().is_none_or(|id| id == board.as_str()) {
    } else {
        return Err(conflict("board binding"));
    }
    if ["sha1", "sha256"].contains(&algorithm.as_str()) {
    } else {
        return Err(error("unknown stored algorithm"));
    }
    if (1..=4096).contains(&count)
        && binding.as_deref() == Some(board.as_str())
        && remote.is_some()
        && reference.as_deref() == Some("refs/heads/quecto/board")
        && source.as_ref().is_some_and(|id| {
            id.len() == (if algorithm == "sha1" { 40 } else { 64 }) && object_id(id).is_ok()
        })
    {
        return Err(HistoryError::Unsupported(
            "populated imported provenance requires B".into(),
        ));
    }
    let native: bool = c.query_row("SELECT NOT EXISTS(SELECT 1 FROM git_objects) AND NOT EXISTS(SELECT 1 FROM revisions WHERE git_commit IS NOT NULL OR git_meta IS NOT NULL)",[],|r|r.get(0)).map_err(error)?;
    if count == 0
        && binding.is_none()
        && algorithm == "sha256"
        && remote.is_none()
        && reference.is_none()
        && source.is_none()
        && native
    {
    } else {
        return Err(error("native provenance shape"));
    }
    let integrity: String = c
        .query_row("PRAGMA integrity_check(1)", [], |r| r.get(0))
        .map_err(error)?;
    if integrity == "ok" {
    } else {
        return Err(error("SQLite integrity"));
    }
    let valid: bool = c.query_row("SELECT count(*)>0 AND min(revision)=1 AND max(revision)=count(*) AND coalesce(min(coalesce(revision>0 AND ((revision=1 AND parent IS NULL) OR (revision>1 AND parent=revision-1)) AND at>=0,0)),0) FROM revisions",[],|r|r.get(0)).map_err(error)?;
    let epochs: bool = c.query_row("SELECT count(*)>0 AND min(epoch)=1 AND max(epoch)=count(*) AND min(from_revision)>0 AND (SELECT from_revision FROM epochs WHERE epoch=1)=1 AND max(from_revision)<=(SELECT max(revision) FROM revisions) AND NOT EXISTS(SELECT 1 FROM epochs e JOIN epochs p ON p.epoch=e.epoch-1 WHERE p.from_revision>e.from_revision) FROM epochs",[],|r|r.get(0)).map_err(error)?;
    // Admit byte lengths in a separate statement: SQL planning must never
    // evaluate substring/cast intermediates on an unbounded historical key.
    // Two canonical positive i64 values plus ':' occupy at most 39 bytes.
    let bounded: bool = c
        .query_row(
            "SELECT coalesce(min(length(CAST(head AS BLOB)) BETWEEN 3 AND 39),0) FROM revisions",
            [],
            |r| r.get(0),
        )
        .map_err(error)?;
    if bounded {
    } else {
        return Err(HistoryError::Bounds("stored head metadata".into()));
    }
    // The key is persisted evidence, not a freely replaceable alias. Resolve
    // its epoch by the indexed integer key, then compare exact canonical bytes
    // and its inclusive held interval. No history rows/names are collected.
    let keys: bool = c.query_row("SELECT count(*)=(SELECT count(*) FROM revisions) FROM revisions r JOIN epochs e ON e.epoch=CAST(substr(r.head,1,instr(r.head,':')-1) AS INTEGER) WHERE r.head=CAST(e.epoch AS TEXT)||':'||CAST(r.revision AS TEXT) AND r.revision>=e.from_revision AND r.revision<=coalesce((SELECT from_revision FROM epochs WHERE epoch>e.epoch ORDER BY epoch LIMIT 1),(SELECT max(revision) FROM revisions))",[],|r|r.get(0)).map_err(error)?;
    if valid && epochs && keys {
        Ok(())
    } else {
        Err(error("native lineage"))
    }
}
impl BoardHistorySnapshot for View {
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>> {
        Ok(None)
    }
    fn is_original(&self, id: &str) -> HistoryResult<bool> {
        object_id(id).map_err(error)?;
        Ok(false)
    }
    fn frontier(&self) -> HistoryResult<BoardHead> {
        Ok(self.frontier.clone())
    }
    fn next_revision(&self, after: u64) -> HistoryResult<Option<HistoryRevision>> {
        if after >= self.tip {
            return Ok(None);
        }
        let seq = i64::try_from(
            after
                .checked_add(1)
                .ok_or_else(|| error("revision overflow"))?,
        )
        .map_err(error)?;
        let revision = self.c.query_row("SELECT revision,parent,root,name,email,message,at FROM revisions WHERE revision=?1 AND length(CAST(root AS BLOB))=64 AND length(CAST(name AS BLOB))<=4096 AND length(CAST(email AS BLOB))<=4096 AND length(CAST(message AS BLOB))<=65536 AND git_commit IS NULL AND git_meta IS NULL",[seq],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,i64>(6)?))).map_err(error)?;
        let (sequence, parent, root, name, email, message, at) = revision;
        object_id(&root).map_err(error)?;
        let writer = Identity { name, email };
        writer.validate("history writer").map_err(error)?;
        line("history message", &message, 200).map_err(error)?;
        Ok(Some(HistoryRevision {
            sequence: u64::try_from(sequence).map_err(error)?,
            parent: parent.map(u64::try_from).transpose().map_err(error)?,
            root,
            writer,
            message,
            at,
            original: None,
        }))
    }
    fn object(&self, id: &str) -> HistoryResult<HistoryObject> {
        use crate::infrastructure::persistence::project_board::board_tree::Reader;
        object_id(id).map_err(error)?;
        let (kind,size):(String,i64)=self.c.query_row("SELECT kind,CASE kind WHEN 'blob' THEN length(bytes) ELSE tree_size END FROM objects WHERE oid=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(error)?;
        match kind.as_str() {
            "blob" if (0..=64 * 1024 * 1024).contains(&size) => {
                let bytes: Vec<u8> = self
                    .c
                    .query_row(
                        "SELECT bytes FROM objects WHERE oid=?1 AND length(bytes)=?2",
                        params![id, size],
                        |r| r.get(0),
                    )
                    .map_err(error)?;
                if storage::blob_id(&bytes) == id {
                } else {
                    return Err(error("native blob hash mismatch"));
                }
                Ok(HistoryObject {
                    oid: id.into(),
                    kind: HistoryKind::Blob,
                    bytes,
                    entries: vec![],
                })
            }
            "tree" if (0..=655360).contains(&size) => {
                let rows = reader::structural(&self.c)
                    .listing(id, 4 * 655360)
                    .map_err(error)?;
                if storage::tree_digest(&rows).map_err(error)?.0 == id {
                } else {
                    return Err(error("native tree hash mismatch"));
                }
                Ok(HistoryObject {
                    oid: id.into(),
                    kind: HistoryKind::Tree,
                    bytes: vec![],
                    entries: rows
                        .into_iter()
                        .map(|e| HistoryEntry {
                            mode: e.mode,
                            name: e.raw,
                            oid: e.oid,
                        })
                        .collect(),
                })
            }
            "blob" | "tree" => Err(HistoryError::Bounds("stored object size".into())),
            _ => Err(error("stored object kind")),
        }
    }
}

fn error(e: impl std::fmt::Debug) -> HistoryError {
    HistoryError::Malformed(format!("{e:?}"))
}
fn conflict(s: &str) -> HistoryError {
    HistoryError::Conflict(s.into())
}
pub(super) fn parse_head(s: &str) -> Result<(i64, i64), BoardStoreError> {
    let Some((e, r)) = s.split_once(':') else {
        return Err(invalid("not an epoch:revision"));
    };
    let parse = |s: &str| -> Result<i64, BoardStoreError> {
        let n: i64 = s.parse().map_err(invalid)?;
        if n > 0 && n.to_string() == s {
            Ok(n)
        } else {
            Err(invalid("noncanonical head"))
        }
    };
    Ok((parse(e)?, parse(r)?))
}
pub(super) fn classify(c: &Connection, head: &BoardHead) -> Result<BoardWrite, BoardStoreError> {
    let (epoch, revision) = parse_head(&head.0)?;
    let (current, _) = storage::current(c)?;
    let (current_epoch, tip) = parse_head(&current.0)?;
    let start: Option<i64> = c
        .query_row(
            "SELECT from_revision FROM epochs WHERE epoch=?1",
            [epoch],
            |r| r.get(0),
        )
        .optional()
        .map_err(unavailable)?;
    let Some(start) = start else {
        return Err(invalid("unknown epoch"));
    };
    if revision >= start {
    } else {
        return Err(invalid("before epoch start"));
    }
    if epoch == current_epoch {
        if revision == tip {
            return Ok(BoardWrite::Written(current));
        }
        if revision < tip {
            return Ok(BoardWrite::Lost);
        }
    } else if epoch < current_epoch {
        let end: i64 = c
            .query_row(
                "SELECT from_revision FROM epochs WHERE epoch=?1",
                [epoch + 1],
                |r| r.get(0),
            )
            .map_err(unavailable)?;
        if revision <= end || revision > tip {
            return Ok(BoardWrite::Lost);
        }
    }
    Err(invalid("impossible epoch/revision"))
}

// RED-only callable seam. GREEN must replace this shell with owned preparation.
impl BoardHistoryStore for SqliteBoardHistoryStore {
    fn prepare_import(&self, _: &LinearHistory) -> HistoryResult<Box<dyn PreparedHistoryImport>> {
        Err(HistoryError::Unavailable(
            "B prepared import not implemented".into(),
        ))
    }
}
