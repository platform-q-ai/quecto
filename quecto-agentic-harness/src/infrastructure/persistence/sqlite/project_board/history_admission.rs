//! Pure DTO admission before the owned import transaction or raw replay.
use super::{error, object_id};
use crate::application::project_board::ports::history::{
    HistoryAlgorithm, HistoryError, HistoryKind, HistoryObject, HistoryResult, LinearHistory,
};
use crate::infrastructure::persistence::project_board::{
    board_tree::raw_entry_bytes, history::commit_links,
};

pub(super) fn projection(o: &HistoryObject, width: usize) -> HistoryResult<()> {
    let mut bytes = o.bytes.as_slice();
    for row in &o.entries {
        let space = bytes
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| error("raw mode"))?;
        let mode = u32::from_str_radix(std::str::from_utf8(&bytes[..space]).map_err(error)?, 8)
            .map_err(error)?;
        let normalized = match mode & 0o170000 {
            0o040000 => "040000",
            0o120000 => "120000",
            0o100000 => {
                if mode & 0o100 != 0 {
                    "100755"
                } else {
                    "100644"
                }
            }
            _ => "160000",
        };
        bytes = &bytes[space + 1..];
        let nul = bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| error("raw name"))?;
        if normalized == row.mode && bytes[..nul] == row.name && nul <= 4096 {
        } else {
            return Err(error("raw tree projection mismatch"));
        }
        bytes = &bytes[nul + 1..];
        let id = bytes
            .get(..width / 2)
            .ok_or_else(|| error("raw oid width"))?;
        if row.oid.len() == width
            && id
                .iter()
                .zip(row.oid.as_bytes().chunks_exact(2))
                .all(|(b, pair)| {
                    std::str::from_utf8(pair)
                        .ok()
                        .and_then(|s| u8::from_str_radix(s, 16).ok())
                        == Some(*b)
                })
        {
        } else {
            return Err(error("raw child identity"));
        }
        bytes = &bytes[width / 2..];
    }
    if bytes.is_empty() {
        Ok(())
    } else {
        Err(error("extra raw tree entries"))
    }
}
pub(super) fn admission(h: &LinearHistory) -> HistoryResult<()> {
    let width = match h.algorithm {
        HistoryAlgorithm::Sha1 => 40,
        HistoryAlgorithm::Sha256 => 64,
    };
    if h.source_ref == "refs/heads/quecto/board"
        && (1..=200).contains(&h.source_remote.len())
        && h.source_head.len() == width
    {
    } else {
        return Err(error("source provenance shape"));
    }
    object_id(&h.source_head).map_err(error)?;
    let mut raw = 0usize;
    let mut metadata = 0usize;
    let mut rows = 0usize;
    let mut charge = |bytes: usize, meta: usize| -> HistoryResult<()> {
        raw = raw
            .checked_add(bytes)
            .ok_or_else(|| error("raw overflow"))?;
        metadata = metadata
            .checked_add(meta)
            .ok_or_else(|| error("metadata overflow"))?;
        if raw <= 128 * 1024 * 1024 && metadata <= 32 * 1024 * 1024 {
            Ok(())
        } else {
            Err(HistoryError::Bounds("import raw/metadata".into()))
        }
    };
    for o in &h.objects {
        let mut meta = 512usize;
        for e in &o.entries {
            object_id(&e.oid).map_err(error)?;
            if e.mode.len() <= 6 {
            } else {
                return Err(error("projection mode length"));
            }
            if e.name.len() <= 4096 {
            } else {
                return Err(HistoryError::Bounds("raw name".into()));
            }
            rows = rows.checked_add(1).ok_or_else(|| error("row overflow"))?;
            meta = meta
                .checked_add(
                    128 + 6
                        + width
                        + e.name
                            .len()
                            .checked_mul(4)
                            .ok_or_else(|| error("name overflow"))?,
                )
                .ok_or_else(|| error("metadata overflow"))?;
        }
        if rows <= 65536 {
        } else {
            return Err(HistoryError::Bounds("import rows".into()));
        }
        if o.oid.len() == width {
        } else {
            return Err(error("import object width"));
        }
        object_id(&o.oid).map_err(error)?;
        if o.kind == HistoryKind::Tree {
            let size = o.entries.iter().try_fold(0usize, |n, e| {
                n.checked_add(raw_entry_bytes(&e.mode, &e.name, &e.oid))
                    .ok_or_else(|| HistoryError::Bounds("normalized tree overflow".into()))
            })?;
            if size <= 655360 && meta - 512 <= 4 * 655360 {
            } else {
                return Err(HistoryError::Bounds("normalized tree admission".into()));
            }
        }
        match o.kind {
            HistoryKind::Blob if o.bytes.len() <= 64 * 1024 * 1024 && o.entries.is_empty() => {}
            HistoryKind::Tree if o.bytes.len() <= 655360 => {
                projection(o, width)?;
            }
            _ => return Err(HistoryError::Bounds("import actual kind/size".into())),
        }
        charge(o.bytes.len(), meta)?;
    }
    for r in &h.revisions {
        r.writer.validate("import writer").map_err(error)?;
        if r.message.len() <= 65536 && r.root.len() == width {
        } else {
            return Err(error("revision metadata"));
        }
        object_id(&r.root).map_err(error)?;
        let original = r
            .original
            .as_ref()
            .ok_or_else(|| error("missing original"))?;
        if original.kind == HistoryKind::Commit
            && original.oid.len() == width
            && original.bytes.len() <= 65536
            && original.entries.is_empty()
        {
        } else {
            return Err(error("original shape"));
        }
        commit_links(&original.bytes, width)?;
        let projection = r
            .writer
            .name
            .len()
            .checked_add(r.writer.email.len())
            .and_then(|n| n.checked_add(r.message.len()))
            .and_then(|n| n.checked_add(256))
            .ok_or_else(|| HistoryError::Bounds("revision projection metadata overflow".into()))?;
        charge(original.bytes.len(), projection.max(512))?;
    }
    Ok(())
}
