//! Reading objects in bulk with `cat-file`: each object's real type and
//! size in one `--batch-check`, and blobs' bytes in one `--batch`, every
//! output bounded by what was asked for.
use super::board_tree::{object_id, unavailable};
use super::git::{Git, failed};
use crate::application::project_board::ports::BoardStoreError;
use std::collections::{BTreeMap, BTreeSet};

/// The most a `cat-file` header or `--batch-check` row takes: a SHA-256
/// name, a type and a size.
const OBJECT_ROW_BYTES: usize = 128;

/// An object's real type and size, by `cat-file --batch-check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub kind: String,
    pub size: usize,
}

/// What `cat-file --batch-check` found for one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Present(Object),
    Missing,
}

/// The real type and size of each object, or that it is missing, from one
/// `cat-file --batch-check`; every object asked for is reported, or it is
/// an error.
pub fn objects<'a>(
    git: &Git,
    oids: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<String, Found>, BoardStoreError> {
    let wanted: BTreeSet<&str> = oids.collect();
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input: String = wanted.iter().map(|oid| format!("{oid}\n")).collect();
    let format = "--batch-check=%(objectname) %(objecttype) %(objectsize)";
    let listing = git
        .ok_up_to(
            &["cat-file", format],
            Some(input.as_bytes()),
            wanted.len() * OBJECT_ROW_BYTES,
        )
        .map_err(unavailable)?;
    let malformed =
        |row: &str| BoardStoreError::Unavailable(format!("unexpected cat-file row {row:?}"));
    let mut found = BTreeMap::new();
    for row in String::from_utf8_lossy(&listing).lines() {
        let (oid, object) = match row.split(' ').collect::<Vec<_>>().as_slice() {
            [oid, "missing"] => (object_id(oid)?, Found::Missing),
            [oid, kind, size] => (
                object_id(oid)?,
                Found::Present(Object {
                    kind: (*kind).into(),
                    size: size.parse().map_err(|_| malformed(row))?,
                }),
            ),
            _ => return Err(malformed(row)),
        };
        found.insert(oid, object);
    }
    match found.keys().map(String::as_str).eq(wanted.iter().copied()) {
        true => Ok(found),
        false => Err(BoardStoreError::Unavailable(format!(
            "cat-file reported {} objects for the {} asked for",
            found.len(),
            wanted.len()
        ))),
    }
}

/// The bytes of each blob, from one `cat-file --batch`; `wanted` names each
/// blob with the size its caller checked first.
pub fn contents(
    git: &Git,
    wanted: &[(&str, usize)],
) -> Result<BTreeMap<String, Vec<u8>>, BoardStoreError> {
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input: String = wanted.iter().map(|(oid, _)| format!("{oid}\n")).collect();
    let limit = wanted
        .iter()
        .map(|(_, size)| size.saturating_add(OBJECT_ROW_BYTES))
        .fold(0usize, usize::saturating_add);
    let args = ["cat-file", "--batch"];
    let output = git
        .local(&args, Some(input.as_bytes()), limit)
        .map_err(unavailable)?;
    if !output.status.success() {
        return Err(unavailable(failed(&args, &output)));
    }
    let malformed = || BoardStoreError::Unavailable("unexpected cat-file --batch output".into());
    let (mut rest, mut found) = (output.stdout.as_slice(), BTreeMap::new());
    while !rest.is_empty() {
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(malformed)?;
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        rest = &rest[end + 1..];
        let fields: Vec<&str> = header.split(' ').collect();
        let [oid, "blob", size] = fields.as_slice() else {
            return Err(BoardStoreError::Unavailable(format!(
                "cat-file read {header:?}"
            )));
        };
        let size: usize = size.parse().map_err(|_| malformed())?;
        let body = rest.get(..size).ok_or_else(malformed)?;
        found.insert(object_id(oid)?, body.to_vec());
        rest = rest.get(size + 1..).ok_or_else(malformed)?;
    }
    for (oid, size) in wanted {
        match found.get(*oid) {
            Some(body) if body.len() == *size => {}
            _ => {
                return Err(BoardStoreError::Unavailable(format!(
                    "cat-file did not return {oid} at its {size} bytes"
                )));
            }
        }
    }
    Ok(found)
}
