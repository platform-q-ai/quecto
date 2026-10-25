//! Pure bounded raw framing admission shared by local capture and replay.
use super::{META, bounds, charge, malformed, oid};
use crate::application::project_board::ports::history::{HistoryError, HistoryResult};

pub(super) fn projection_budget(mut bytes: &[u8], width: usize) -> HistoryResult<(usize, usize)> {
    let mut rows = 0;
    let mut metadata = 0;
    while let [_, ..] = bytes {
        let space = bytes
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| malformed("tree mode frame"))?;
        if space > 0 && bytes[..space].iter().all(|b| (b'0'..=b'7').contains(b)) {
        } else {
            return Err(malformed("tree mode"));
        }
        bytes = &bytes[space + 1..];
        let nul = bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| malformed("tree name frame"))?;
        if nul <= 4096 {
        } else {
            return Err(bounds("raw name"));
        }
        let length = nul
            .checked_mul(4)
            .and_then(|n| n.checked_add(128 + 6 + width))
            .ok_or_else(|| bounds("projection metadata"))?;
        charge(&mut rows, 1, 65536)?;
        charge(&mut metadata, length, META)?;
        bytes = bytes
            .get(nul + 1 + width / 2..)
            .ok_or_else(|| malformed("tree child frame"))?;
    }
    Ok((rows, metadata))
}

/// Commit metadata is retained raw; only structural and required-header framing
/// is interpreted here. Encodings, signatures and extension headers are opaque.
pub(crate) fn commit_links(bytes: &[u8], width: usize) -> HistoryResult<(String, Option<String>)> {
    let end = bytes
        .windows(2)
        .position(|s| s == b"\n\n")
        .ok_or_else(|| malformed("commit header"))?;
    let lines: Vec<_> = bytes[..end].split(|b| *b == b'\n').collect();
    if lines.first().is_some_and(|l| l.starts_with(b"tree "))
        && lines.iter().all(|l| {
            l.starts_with(b" ")
                || l.iter()
                    .position(|b| *b == b' ')
                    .is_some_and(|n| n > 0 && l[..n].iter().all(|b| (33..=126).contains(b)))
        })
    {
    } else {
        return Err(malformed("commit header framing"));
    }
    for required in [b"author ".as_slice(), b"committer ".as_slice()] {
        let identities: Vec<_> = lines
            .iter()
            .filter_map(|l| l.strip_prefix(required))
            .collect();
        if let [identity] = identities.as_slice() {
            let fields: Vec<_> = identity.rsplitn(3, |b| *b == b' ').collect();
            if let [zone, time, name] = fields.as_slice() {
                if zone.len() == 5
                    && matches!(zone[0], b'+' | b'-')
                    && zone[1..].iter().all(u8::is_ascii_digit)
                    && std::str::from_utf8(time)
                        .ok()
                        .and_then(|s| s.parse::<i64>().ok())
                        .is_some()
                    && name.ends_with(b">")
                    && name
                        .windows(2)
                        .position(|b| b == b" <")
                        .is_some_and(|n| n > 0 && n + 3 < name.len())
                {
                    continue;
                }
            }
        }
        return Err(malformed("missing/invalid required identity header"));
    }
    let mut root = None;
    let mut parent = None;
    for line in lines {
        if let Some(value) = line.strip_prefix(b"tree ") {
            if root.is_none() {
            } else {
                return Err(malformed("duplicate tree header"));
            }
            let id = std::str::from_utf8(value).map_err(malformed)?;
            oid(id, width)?;
            root = Some(id.to_owned());
        }
        if let Some(value) = line.strip_prefix(b"parent ") {
            if parent.is_none() {
            } else {
                return Err(HistoryError::Unsupported("linear history required".into()));
            }
            let id = std::str::from_utf8(value).map_err(malformed)?;
            oid(id, width)?;
            parent = Some(id.to_owned());
        }
    }
    Ok((root.ok_or_else(|| malformed("missing root"))?, parent))
}
