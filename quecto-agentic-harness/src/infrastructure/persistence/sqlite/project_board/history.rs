//! Native epoch fencing and snapshot-only history adapter.
use super::*;
use crate::application::project_board::ports::history::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::infrastructure::persistence::project_board::{
    board_tree::Reader,
    history::{commit_links, verify_raw},
};
use std::path::PathBuf;
#[path = "history_admission.rs"]
mod admission;
use admission::{admission, projection};

pub struct SqliteBoardHistoryStore {
    path: PathBuf,
    writer: Identity,
    board: BoardId,
    /// A guard kept for the store's life: the state directory's lock (#2741).
    _held: Option<Box<dyn std::any::Any + Send + Sync>>,
}
impl SqliteBoardHistoryStore {
    pub fn new(path: PathBuf, writer: Identity, board: BoardId) -> Self {
        Self {
            path,
            writer,
            board,
            _held: None,
        }
    }
    /// This store, keeping `guard` until it is dropped.
    pub fn holding(self, guard: Box<dyn std::any::Any + Send + Sync>) -> Self {
        Self {
            _held: Some(guard),
            ..self
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
            algorithm: None,
        };
        storage::recognize_on(&view.c, false)
            .map_err(|e| HistoryError::Unsupported(format!("{e:?}")))?;
        view.algorithm = validate_meta(&view.c, &self.board)?;
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
    algorithm: Option<HistoryAlgorithm>,
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
    validate_lineage(c)
}
fn validate_lineage(c: &Connection) -> HistoryResult<()> {
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
        Ok(self.algorithm)
    }
    fn is_original(&self, id: &str) -> HistoryResult<bool> {
        object_id(id).map_err(error)?;
        self.c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM git_objects WHERE oid=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(error)
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
        if self.algorithm.is_some() {
            if let Some(original) = imported_revision(self, seq)? {
                return Ok(Some(original));
            }
        }
        let revision = self.c.query_row("SELECT revision,parent,root,name,email,message,at FROM revisions WHERE revision=?1 AND length(CAST(root AS BLOB))<=64 AND length(CAST(name AS BLOB))<=4096 AND length(CAST(email AS BLOB))<=4096 AND length(CAST(message AS BLOB))<=65536 AND git_commit IS NULL AND git_meta IS NULL",[seq],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,i64>(6)?))).map_err(error)?;
        let (sequence, parent, root, name, email, message, at) = revision;
        object_id(&root).map_err(error)?;
        if root.len() == 64 || (self.algorithm.is_some() && self.is_original(&root)?) {
        } else {
            return Err(error("native root width/membership"));
        }
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
        if self.is_original(id)? {
            return imported_object(&self.c, id);
        }
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

fn imported_object(c: &Connection, id: &str) -> HistoryResult<HistoryObject> {
    let (kind,size,raw):(String,i64,Option<i64>) = c.query_row("SELECT kind,CASE kind WHEN 'blob' THEN length(bytes) ELSE tree_size END,(SELECT length(raw_tree) FROM git_objects WHERE oid=objects.oid) FROM objects WHERE oid=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(error)?;
    let (kind, bytes, entries) = match kind.as_str() {
        "blob" if (0..=64 * 1024 * 1024).contains(&size) && raw.is_none() => {
            let bytes = c
                .query_row("SELECT bytes FROM objects WHERE oid=?1", [id], |r| r.get(0))
                .map_err(error)?;
            (HistoryKind::Blob, bytes, vec![])
        }
        "tree"
            if (0..=655360).contains(&size) && raw.is_some_and(|n| (0..=655360).contains(&n)) =>
        {
            let bytes = c
                .query_row("SELECT raw_tree FROM git_objects WHERE oid=?1", [id], |r| {
                    r.get(0)
                })
                .map_err(error)?;
            let rows = reader::structural(c)
                .listing(id, 4 * 655360)
                .map_err(|e| error(format!("{e:?}")))?;
            (
                HistoryKind::Tree,
                bytes,
                rows.into_iter()
                    .map(|e| HistoryEntry {
                        mode: e.mode,
                        name: e.raw,
                        oid: e.oid,
                    })
                    .collect(),
            )
        }
        _ => return Err(error("original object kind/size/membership shape")),
    };
    let o = HistoryObject {
        oid: id.into(),
        kind,
        bytes,
        entries,
    };
    verify_raw(&o, id.len())?;
    if kind == HistoryKind::Tree {
        projection(&o, id.len())?;
    }
    Ok(o)
}
fn imported_revision(view: &View, seq: i64) -> HistoryResult<Option<HistoryRevision>> {
    type Row = (
        i64,
        Option<i64>,
        String,
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<Vec<u8>>,
    );
    let row:Row=view.c.query_row("SELECT revision,parent,root,name,email,message,at,git_commit,git_meta FROM revisions WHERE revision=?1 AND length(root)<=64 AND length(CAST(name AS BLOB))<=4096 AND length(CAST(email AS BLOB))<=4096 AND length(CAST(message AS BLOB))<=65536 AND (git_meta IS NULL OR length(git_meta)<=65536) AND (git_commit IS NULL OR length(git_commit)<=64)",[seq],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(error)?;
    let (sequence, parent, root, name, email, message, at, id, body) = row;
    let original = match (id, body) {
        (None, None) => return Ok(None),
        (Some(oid), Some(bytes)) => HistoryObject {
            oid,
            kind: HistoryKind::Commit,
            bytes,
            entries: vec![],
        },
        _ => return Err(error("original commit shape")),
    };
    let width = match view.algorithm {
        Some(HistoryAlgorithm::Sha1) => 40,
        Some(HistoryAlgorithm::Sha256) => 64,
        _ => return Err(error("missing original algorithm")),
    };
    if original.oid.len() == width && root.len() == width {
    } else {
        return Err(error("original commit width"));
    }
    verify_raw(&original, width)?;
    let (tree, git_parent) = commit_links(&original.bytes, width)?;
    let expected: Option<String> = if seq == 1 {
        None
    } else {
        view.c
            .query_row(
                "SELECT git_commit FROM revisions WHERE revision=?1",
                [seq - 1],
                |r| r.get(0),
            )
            .map_err(error)?
    };
    if tree == root && git_parent == expected {
    } else {
        return Err(error("original root/parent mismatch"));
    }
    Ok(Some(HistoryRevision {
        sequence: u64::try_from(sequence).map_err(error)?,
        parent: parent.map(u64::try_from).transpose().map_err(error)?,
        root,
        writer: Identity { name, email },
        message,
        at,
        original: Some(original),
    }))
}
fn validate_meta(c: &Connection, board: &BoardId) -> HistoryResult<Option<HistoryAlgorithm>> {
    type Meta = (
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let (binding,algorithm,remote,reference,source,count):Meta=c.query_row("SELECT board_id,algorithm,source_remote,source_ref,source_head,imported_count FROM history_meta WHERE id=1 AND (board_id IS NULL OR length(CAST(board_id AS BLOB))=16) AND length(CAST(algorithm AS BLOB))<=6 AND (source_remote IS NULL OR length(CAST(source_remote AS BLOB)) BETWEEN 1 AND 200) AND (source_ref IS NULL OR length(CAST(source_ref AS BLOB))<=64) AND (source_head IS NULL OR length(CAST(source_head AS BLOB))<=64) AND imported_count BETWEEN 0 AND 4096",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(error)?;
    if count == 0 {
        validate_native(c, board)?;
        return Ok(None);
    }
    if binding.as_deref() == Some(board.as_str()) {
    } else {
        return Err(conflict("board binding"));
    }
    let algorithm = match algorithm.as_str() {
        "sha1" => HistoryAlgorithm::Sha1,
        "sha256" => HistoryAlgorithm::Sha256,
        _ => return Err(error("stored original algorithm")),
    };
    let width = match algorithm {
        HistoryAlgorithm::Sha1 => 40,
        HistoryAlgorithm::Sha256 => 64,
    };
    let (actual,first,last):(i64,Option<i64>,Option<i64>)=c.query_row("SELECT count(*),min(revision),max(revision) FROM revisions WHERE git_commit IS NOT NULL AND git_meta IS NOT NULL",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(error)?;
    if actual == count
        && first == Some(1)
        && last == Some(count)
        && remote.is_some()
        && reference.as_deref() == Some("refs/heads/quecto/board")
    {
    } else {
        return Err(HistoryError::Unsupported(
            "original prefix/provenance mismatch".into(),
        ));
    }
    let source = source.ok_or_else(|| error("source frontier missing"))?;
    if source.len() == width {
    } else {
        return Err(error("source width"));
    }
    object_id(&source).map_err(error)?;
    // Compare a bounded provenance value as a SQL scalar: never decode an
    // arbitrary stored original ID before the prefix width/type admission.
    let matches: bool = c
        .query_row(
            "SELECT count(*)=1 FROM revisions WHERE revision=?1 AND typeof(git_commit)='text' AND length(CAST(git_commit AS BLOB))=?2 AND git_commit=?3",
            params![count, width as i64, source],
            |r| r.get(0),
        )
        .map_err(error)?;
    if matches {
    } else {
        return Err(error("source frontier mismatch"));
    }
    let rooted:i64=c.query_row("SELECT count(*) FROM revisions r JOIN git_objects g ON r.root=g.oid JOIN objects o ON o.oid=g.oid WHERE r.revision<=?1 AND o.kind='tree' AND length(CAST(r.root AS BLOB))=?2 AND length(CAST(r.git_commit AS BLOB))=?2 AND typeof(r.git_meta)='blob' AND length(r.git_meta)<=65536 AND length(CAST(r.name AS BLOB))<=4096 AND length(CAST(r.email AS BLOB))<=4096 AND length(CAST(r.message AS BLOB))<=65536",params![count,width as i64],|r|r.get(0)).map_err(error)?;
    if rooted == count {
    } else {
        return Err(error("original root/commit membership"));
    }
    let members: i64 = c
        .query_row("SELECT count(*) FROM git_objects", [], |r| r.get(0))
        .map_err(error)?;
    let valid:i64=c.query_row("SELECT count(*) FROM git_objects AS g JOIN objects AS o USING(oid) WHERE length(g.oid)=?1 AND ((o.kind='blob' AND g.raw_tree IS NULL) OR (o.kind='tree' AND g.raw_tree IS NOT NULL))",[width as i64],|r|r.get(0)).map_err(error)?;
    let partial: i64 = c
        .query_row(
            "SELECT count(*) FROM revisions WHERE (git_commit IS NULL)<>(git_meta IS NULL)",
            [],
            |r| r.get(0),
        )
        .map_err(error)?;
    if (1..=32768).contains(&members) && valid == members && partial == 0 {
    } else {
        return Err(error("original membership shape"));
    }
    let raw:i64=c.query_row("SELECT coalesce(sum(CASE o.kind WHEN 'blob' THEN length(o.bytes) ELSE length(g.raw_tree) END),0)+(SELECT coalesce(sum(length(git_meta)),0) FROM revisions WHERE git_commit IS NOT NULL) FROM git_objects g JOIN objects o USING(oid)",[],|r|r.get(0)).map_err(error)?;
    let (rows,metadata):(i64,i64)=c.query_row("SELECT count(*),coalesce(sum(128+6+?1+4*length(e.raw_name)),0) FROM tree_entries e JOIN git_objects g ON e.tree=g.oid",[width as i64],|r|Ok((r.get(0)?,r.get(1)?))).map_err(error)?;
    let revisions_meta:i64=c.query_row("SELECT coalesce(sum(max(512,256+length(CAST(name AS BLOB))+length(CAST(email AS BLOB))+length(CAST(message AS BLOB)))),0) FROM revisions WHERE git_commit IS NOT NULL",[],|r|r.get(0)).map_err(error)?;
    if (0..=128 * 1024 * 1024).contains(&raw)
        && (0..=65536).contains(&rows)
        && metadata
            .checked_add(members * 512)
            .and_then(|n| n.checked_add(revisions_meta))
            .is_some_and(|n| (0..=32 * 1024 * 1024).contains(&n))
    {
    } else {
        return Err(HistoryError::Bounds("stored original resources".into()));
    }
    validate_lineage(c)?;
    Ok(Some(algorithm))
}

struct Prepared {
    view: View,
    report: HistoryImportReport,
}
impl BoardHistorySnapshot for Prepared {
    fn is_original(&self, id: &str) -> HistoryResult<bool> {
        self.view.is_original(id)
    }
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>> {
        self.view.algorithm()
    }
    fn frontier(&self) -> HistoryResult<BoardHead> {
        self.view.frontier()
    }
    fn next_revision(&self, n: u64) -> HistoryResult<Option<HistoryRevision>> {
        self.view.next_revision(n)
    }
    fn object(&self, id: &str) -> HistoryResult<HistoryObject> {
        self.view.object(id)
    }
}
impl PreparedHistoryImport for Prepared {
    fn commit(self: Box<Self>) -> HistoryResult<HistoryImportReport> {
        self.view.c.execute_batch("COMMIT").map_err(error)?;
        Ok(self.report.clone())
    }
}
impl BoardHistoryStore for SqliteBoardHistoryStore {
    fn prepare_import(&self, h: &LinearHistory) -> HistoryResult<Box<dyn PreparedHistoryImport>> {
        self.writer.validate("writer").map_err(error)?;
        if (1..=4096).contains(&h.revisions.len()) && h.objects.len() <= 32768 {
        } else {
            return Err(HistoryError::Bounds("import counts".into()));
        }
        admission(h)?;
        let c = storage::open(&self.path).map_err(error)?;
        c.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
        let mut view = View {
            c,
            frontier: BoardHead(format!("1:{}", h.revisions.len())),
            tip: u64::try_from(h.revisions.len()).map_err(error)?,
            algorithm: Some(h.algorithm),
        };
        for (table, _) in storage::TABLES {
            let n: i64 = view
                .c
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .map_err(error)?;
            if n == 0 {
            } else {
                return Err(conflict("import requires initialized empty store"));
            }
        }
        let mut writer = storage::ObjectWriter::new(&view.c);
        for o in &h.objects {
            writer.imported(o).map_err(error)?;
        }
        for (i, r) in h.revisions.iter().enumerate() {
            if r.sequence == i as u64 + 1 && r.parent == if i == 0 { None } else { Some(i as u64) }
            {
            } else {
                return Err(error("import lineage"));
            }
            let original = r
                .original
                .as_ref()
                .ok_or_else(|| error("missing original commit"))?;
            if original.kind == HistoryKind::Commit && original.bytes.len() <= 65536 {
            } else {
                return Err(error("original kind/size"));
            }
            writer
                .insert(
                    "INSERT INTO revisions VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        format!("1:{}", r.sequence),
                        r.root,
                        r.writer.name,
                        r.writer.email,
                        r.sequence as i64,
                        r.parent.map(|p| p as i64),
                        r.message,
                        r.at,
                        original.oid,
                        original.bytes
                    ],
                )
                .map_err(error)?;
            use crate::infrastructure::persistence::project_board::board_tree::{
                LeftOnRemote, read_root,
            };
            read_root(
                &reader::structural(&view.c),
                &r.root,
                LIMITS,
                &LeftOnRemote::default(),
            )
            .map_err(|e| error(format!("{e:?}")))?;
        }
        writer
            .insert(
                "INSERT INTO epochs VALUES (1,1,'import',?1)",
                [h.revisions.last().map_or(0, |r| r.at)],
            )
            .map_err(error)?;
        writer
            .insert(
                "INSERT INTO history_meta VALUES (1,?1,?2,?3,?4,?5,?6)",
                params![
                    self.board.as_str(),
                    match h.algorithm {
                        HistoryAlgorithm::Sha1 => "sha1",
                        HistoryAlgorithm::Sha256 => "sha256",
                    },
                    h.source_remote,
                    h.source_ref,
                    h.source_head,
                    h.revisions.len() as i64
                ],
            )
            .map_err(error)?;
        writer
            .insert("INSERT INTO state VALUES (1,?1)", [&view.frontier.0])
            .map_err(error)?;
        view.algorithm = validate_meta(&view.c, &self.board)?;
        let report = HistoryImportReport {
            revisions: view.tip,
            head: view.frontier.clone(),
            source_head: h.source_head.clone(),
        };
        // The returned owner keeps BEGIN IMMEDIATE open through shared export proof.
        view.tip = report.revisions;
        Ok(Box::new(Prepared { view, report }))
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
