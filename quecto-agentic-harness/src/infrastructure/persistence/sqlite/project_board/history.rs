//! Native epoch fencing and snapshot-only history adapter.
use super::*;
use crate::application::project_board::ports::history::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use std::path::PathBuf;

pub struct SqliteBoardHistoryStore {
    path: PathBuf,
    _writer: Identity,
    _board: BoardId,
}
impl SqliteBoardHistoryStore {
    pub fn new(path: PathBuf, writer: Identity, board: BoardId) -> Self {
        Self {
            path,
            _writer: writer,
            _board: board,
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
        Err(HistoryError::Unavailable(
            "B3 pinned snapshot is not implemented (RED compilation shell)".into(),
        ))
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
