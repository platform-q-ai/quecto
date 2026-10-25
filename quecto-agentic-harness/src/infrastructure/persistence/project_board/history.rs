//! Real deterministic Git serialization and captured-lease publication.
use super::board_tree::{Found, Reader};
use super::git::{Git, GitEnvironment};
use crate::application::project_board::ports::history::*;
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
#[path = "history_decode.rs"]
mod decode;
pub(crate) use decode::commit_links;
use decode::projection_budget;

fn malformed(why: impl ToString) -> HistoryError {
    HistoryError::Malformed(why.to_string())
}
fn unavailable(why: impl ToString) -> HistoryError {
    HistoryError::Unavailable(why.to_string())
}
fn bounds(why: &str) -> HistoryError {
    HistoryError::Bounds(why.into())
}
fn git(repo: &std::path::Path) -> HistoryResult<Git> {
    Git::reader(repo, GitEnvironment::of_process())
        .map_err(|e| HistoryError::Unavailable(e.to_string()))
}
fn kind(k: HistoryKind) -> &'static str {
    match k {
        HistoryKind::Blob => "blob",
        HistoryKind::Tree => "tree",
        HistoryKind::Commit => "commit",
    }
}
fn oid(id: &str, width: usize) -> HistoryResult<()> {
    match id.len() == width
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        true => Ok(()),
        false => Err(malformed("invalid object name")),
    }
}
fn charge(value: &mut usize, n: usize, cap: usize) -> HistoryResult<()> {
    *value = value
        .checked_add(n)
        .ok_or_else(|| bounds("counter overflow"))?;
    match *value <= cap {
        true => Ok(()),
        false => Err(bounds("history admission exceeded")),
    }
}
pub struct GitBoardHistorySink {
    repo: PathBuf,
    target_ref: String,
}
impl GitBoardHistorySink {
    pub fn new(repo: PathBuf, target_ref: String) -> Self {
        Self { repo, target_ref }
    }
}
impl BoardHistorySink for GitBoardHistorySink {
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>> {
        match git(&self.repo)?
            .text(&["rev-parse", "--show-object-format"], None)
            .map_err(unavailable)?
            .as_str()
        {
            "sha1" => Ok(Some(HistoryAlgorithm::Sha1)),
            "sha256" => Ok(Some(HistoryAlgorithm::Sha256)),
            _ => Err(HistoryError::Unsupported(
                "destination object format".into(),
            )),
        }
    }
    fn raw(
        &self,
        k: HistoryKind,
        bytes: &[u8],
        expected: Option<&str>,
        dry: bool,
    ) -> HistoryResult<String> {
        let cap = match k {
            HistoryKind::Blob => 64 * 1024 * 1024,
            HistoryKind::Tree => 655360,
            HistoryKind::Commit => 65536,
        };
        if bytes.len() <= cap {
        } else {
            return Err(bounds("sink object size"));
        }
        let git = git(&self.repo)?;
        let args = if dry {
            vec!["hash-object", "--literally", "-t", kind(k), "--stdin"]
        } else {
            vec!["hash-object", "--literally", "-w", "-t", kind(k), "--stdin"]
        };
        // Hash first, before any write, including expected-OID verification.
        let hash = git
            .text(
                &["hash-object", "--literally", "-t", kind(k), "--stdin"],
                Some(bytes),
            )
            .map_err(unavailable)?;
        if expected.is_none_or(|expected| expected == hash) {
        } else {
            return Err(malformed("expected raw object hash mismatch"));
        }
        if dry {
        } else {
            let written = git.text(&args, Some(bytes)).map_err(unavailable)?;
            if written == hash {
            } else {
                return Err(malformed("written hash mismatch"));
            }
        }
        Ok(hash)
    }
    fn tree(&self, entries: &[HistoryEntry], dry: bool) -> HistoryResult<String> {
        let git = git(&self.repo)?;
        let width = match git
            .text(&["rev-parse", "--show-object-format"], None)
            .map_err(malformed)?
            .as_str()
        {
            "sha1" => 40,
            "sha256" => 64,
            _ => return Err(malformed("format")),
        };
        if entries.len() <= 65536 {
        } else {
            return Err(bounds("native tree entry count"));
        }
        let mut size = 0usize;
        for e in entries {
            if ["100644", "100755", "120000", "040000", "160000"].contains(&e.mode.as_str())
                && (1..=4096).contains(&e.name.len())
                && e.name
                    .iter()
                    .all(|b| (1..=46).contains(b) || (48..=255).contains(b))
            {
            } else {
                return Err(malformed("native tree entry"));
            }
            oid(&e.oid, width)?;
            charge(
                &mut size,
                e.mode.trim_start_matches('0').len() + 1 + e.name.len() + 1 + width / 2,
                655360,
            )?;
        }
        let mut names = BTreeSet::new();
        if entries.iter().all(|e| names.insert(e.name.as_slice())) {
        } else {
            return Err(malformed("duplicate native tree name"));
        }
        let mut entries: Vec<_> = entries.iter().collect();
        entries.sort_by(|a, b| {
            a.name
                .iter()
                .copied()
                .chain((a.mode == "040000").then_some(b'/'))
                .cmp(
                    b.name
                        .iter()
                        .copied()
                        .chain((b.mode == "040000").then_some(b'/')),
                )
        });
        let mut bytes = Vec::with_capacity(size);
        for e in entries {
            bytes.extend_from_slice(e.mode.trim_start_matches('0').as_bytes());
            bytes.push(b' ');
            bytes.extend_from_slice(&e.name);
            bytes.push(0);
            for i in (0..width).step_by(2) {
                bytes.push(u8::from_str_radix(&e.oid[i..i + 2], 16).map_err(malformed)?);
            }
        }
        self.raw(HistoryKind::Tree, &bytes, None, dry)
    }
    fn commit(&self, revision: HistoryCommit<'_>, dry: bool) -> HistoryResult<String> {
        let HistoryCommit {
            tree,
            parent,
            writer,
            message,
            at,
        } = revision;
        writer.validate("history writer").map_err(malformed)?;
        if [40, 64].contains(&tree.len()) {
        } else {
            return Err(malformed("commit tree width"));
        }
        oid(tree, tree.len())?;
        if let Some(parent) = parent {
            oid(parent, tree.len())?;
        }
        if at >= 0 && message.len() <= 65536 && message.bytes().all(|b| (1..=255).contains(&b)) {
        } else {
            return Err(malformed("commit metadata"));
        }
        let time = at.to_string();
        let identity = writer
            .name
            .len()
            .checked_add(writer.email.len())
            .and_then(|n| n.checked_add(time.len()))
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| bounds("commit metadata overflow"))?;
        let size = (6 + tree.len())
            .checked_add(parent.map_or(0, |p| 8 + p.len()))
            .and_then(|n| n.checked_add(39 + identity + 2))
            .and_then(|n| n.checked_add(message.len()))
            .ok_or_else(|| bounds("commit size overflow"))?;
        if size <= 65536 {
        } else {
            return Err(bounds("commit size"));
        }
        let mut body = String::with_capacity(size);
        body.push_str(&format!("tree {tree}\n"));
        if let Some(parent) = parent {
            body.push_str(&format!("parent {parent}\n"));
        }
        body.push_str(&format!(
            "author {} <{}> {at} +0000\ncommitter {} <{}> {at} +0000\n\n{message}\n",
            writer.name, writer.email, writer.name, writer.email
        ));
        assert_eq!(body.len(), size, "checked commit framing size");
        self.raw(HistoryKind::Commit, body.as_bytes(), None, dry)
    }
    fn publish(&self, expected: Option<&str>, new: &str) -> HistoryResult<()> {
        let git = git(&self.repo)?;
        if self.target_ref.len() <= 1024
            && self.target_ref.starts_with("refs/heads/")
            && git
                .local(&["check-ref-format", &self.target_ref], None, 1024)
                .map_err(malformed)?
                .status
                .success()
        {
        } else {
            return Err(malformed("destination ref"));
        }
        let width = match git
            .text(&["rev-parse", "--show-object-format"], None)
            .map_err(malformed)?
            .as_str()
        {
            "sha1" => 40,
            "sha256" => 64,
            _ => return Err(malformed("destination format")),
        };
        oid(new, width)?;
        if let Some(old) = expected {
            oid(old, width)?;
        }
        let absent = "0".repeat(width);
        let result = git.local(
            &[
                "update-ref",
                "--no-deref",
                &self.target_ref,
                new,
                expected.unwrap_or(&absent),
            ],
            None,
            65536,
        );
        match result {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => {
                let failure = super::git::failed(
                    &[
                        "update-ref",
                        "--no-deref",
                        &self.target_ref,
                        new,
                        expected.unwrap_or(&absent),
                    ],
                    &out,
                );
                let detail = format!(
                    "expected={expected:?} new={new}; verified objects may remain unreachable: {failure}"
                );
                let observed = git.local(
                    &["rev-parse", "--verify", "--quiet", &self.target_ref],
                    None,
                    1024,
                );
                let observed = match observed {
                    Ok(out) if out.status.success() => {
                        let id = std::str::from_utf8(&out.stdout)
                            .map_err(|_| HistoryError::PublicationUnknown(detail.clone()))?
                            .trim();
                        oid(id, width)
                            .map_err(|_| HistoryError::PublicationUnknown(detail.clone()))?;
                        Some(id.to_owned())
                    }
                    Ok(out) if out.status.code() == Some(1) => None,
                    _ => return Err(HistoryError::PublicationUnknown(detail)),
                };
                if observed.as_deref() == Some(new) {
                    Err(HistoryError::PublicationUnknown(detail))
                } else if observed.as_deref() == expected {
                    Err(HistoryError::Unavailable(detail))
                } else {
                    Err(HistoryError::Conflict(format!(
                        "{detail}; observed={observed:?}"
                    )))
                }
            }
            Err(e) => Err(HistoryError::PublicationUnknown(format!(
                "expected={expected:?} new={new}: {e}"
            ))),
        }
    }
    fn finish_dry_run(&self) -> HistoryResult<()> {
        Ok(())
    }
}

const BOARD: &str = "refs/heads/quecto/board";
const RAW: usize = 128 * 1024 * 1024;
const META: usize = 32 * 1024 * 1024;

/// Exact Git framing, independent of declared tree modes and repository filters.
pub(crate) fn verify_raw(object: &HistoryObject, width: usize) -> HistoryResult<()> {
    let frame = format!("{} {}\0", kind(object.kind), object.bytes.len());
    let actual = match width {
        40 => {
            let mut hash = sha1::Sha1::new();
            hash.update(frame.as_bytes());
            hash.update(&object.bytes);
            format!("{:x}", hash.finalize())
        }
        64 => {
            let mut hash = sha2::Sha256::new();
            hash.update(frame.as_bytes());
            hash.update(&object.bytes);
            format!("{:x}", hash.finalize())
        }
        _ => return Err(malformed("object format")),
    };
    if actual == object.oid {
        Ok(())
    } else {
        Err(malformed("raw hash mismatch"))
    }
}

struct Capture<'a> {
    git: &'a Git,
    width: usize,
    raw: usize,
    metadata: usize,
    rows: usize,
}
impl Capture<'_> {
    /// Metadata precedes bodies. The retained runner frame and body copies are
    /// separately bounded by RAW + 128 bytes per admitted object, not RSS.
    fn read(
        &mut self,
        ids: &[String],
        expected: Option<HistoryKind>,
    ) -> HistoryResult<Vec<HistoryObject>> {
        for id in ids {
            oid(id, self.width)?;
        }
        if (1..=32768).contains(&ids.len()) {
        } else {
            return Err(bounds("batch count"));
        }
        let refs: Vec<_> = ids.iter().map(String::as_str).collect();
        let metadata = self
            .git
            .metadata(&refs)
            .map_err(|e| malformed(format!("{e:?}")))?;
        let mut plans = Vec::with_capacity(ids.len());
        let mut frame_size = 0usize;
        for id in ids {
            let Some(Found::Present(meta)) = metadata.get(id) else {
                return Err(malformed("missing local object"));
            };
            let (k, limit) = match meta.kind.as_str() {
                "blob" => (HistoryKind::Blob, 64 * 1024 * 1024),
                "tree" => (HistoryKind::Tree, 655360),
                "commit" => (HistoryKind::Commit, 65536),
                _ => return Err(HistoryError::Unsupported("actual object type".into())),
            };
            if expected.is_none_or(|e| e == k) {
            } else {
                return Err(malformed("unexpected structural object kind"));
            }
            if meta.size <= limit {
            } else {
                return Err(bounds("object size"));
            }
            charge(&mut self.raw, meta.size, RAW)?;
            charge(&mut self.metadata, 512, META)?;
            let header = format!("{id} {} {}\n", kind(k), meta.size);
            let size = header
                .len()
                .checked_add(meta.size)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| bounds("frame overflow"))?;
            charge(&mut frame_size, size, RAW + 32768 * 128)?;
            plans.push((id, k, meta.size, header));
        }
        let input: String = ids.iter().map(|id| format!("{id}\n")).collect();
        let frame = self
            .git
            .ok_up_to(&["cat-file", "--batch"], Some(input.as_bytes()), frame_size)
            .map_err(malformed)?;
        if frame.len() == frame_size {
        } else {
            return Err(malformed("batch frame length"));
        }
        let mut rest = frame.as_slice();
        let mut result = Vec::with_capacity(ids.len());
        for (id, k, size, header) in plans {
            let body = rest
                .strip_prefix(header.as_bytes())
                .ok_or_else(|| malformed("raw frame header"))?;
            let bytes = body
                .get(..size)
                .ok_or_else(|| malformed("raw body length"))?;
            if body.get(size) == Some(&b'\n') {
            } else {
                return Err(malformed("raw frame terminator"));
            }
            let mut object = HistoryObject {
                oid: id.clone(),
                kind: k,
                bytes: bytes.to_vec(),
                entries: vec![],
            };
            verify_raw(&object, self.width)?;
            self.project(&mut object)?;
            rest = &body[size + 1..];
            result.push(object);
        }
        assert!(
            rest.is_empty(),
            "checked batch frames consume exact bounded output"
        );
        Ok(result)
    }
}

impl Capture<'_> {
    fn closure(
        &mut self,
        roots: BTreeSet<String>,
    ) -> HistoryResult<BTreeMap<String, HistoryObject>> {
        let mut objects = BTreeMap::new();
        let mut pending = roots.clone();
        let mut depth = 0usize;
        while pending.first().is_some() {
            if objects
                .len()
                .checked_add(pending.len())
                .is_some_and(|n| n <= 32768)
            {
            } else {
                return Err(bounds("object count"));
            }
            let ids: Vec<_> = pending.into_iter().collect();
            let batch = self.read(&ids, None)?;
            pending = BTreeSet::new();
            for object in &batch {
                if matches!(object.kind, HistoryKind::Blob | HistoryKind::Tree) {
                } else {
                    return Err(HistoryError::Unsupported("non-gitlink commit".into()));
                }
                if roots.contains(&object.oid) && object.kind == HistoryKind::Tree {
                } else if roots.contains(&object.oid) {
                    return Err(malformed("revision root is not tree"));
                }
                if object.kind == HistoryKind::Blob || depth <= 128 {
                } else {
                    return Err(bounds("tree depth"));
                }
                for row in &object.entries {
                    if row.mode == "160000" {
                        continue;
                    }
                    if objects.contains_key(&row.oid)
                        || ids.binary_search(&row.oid).is_ok()
                        || pending.contains(&row.oid)
                    {
                    } else {
                        if objects
                            .len()
                            .checked_add(ids.len())
                            .and_then(|n| n.checked_add(pending.len()))
                            .is_some_and(|n| n < 32768)
                        {
                        } else {
                            return Err(bounds("object count"));
                        }
                        pending.insert(row.oid.clone());
                    }
                }
            }
            for object in batch {
                objects.insert(object.oid.clone(), object);
            }
            if objects
                .len()
                .checked_add(pending.len())
                .is_some_and(|n| n <= 32768)
            {
            } else {
                return Err(bounds("object count"));
            }
            depth = depth
                .checked_add(1)
                .ok_or_else(|| bounds("depth overflow"))?;
        }
        let mut heights = BTreeMap::new();
        for root in roots {
            height(&root, &objects, &mut heights, &mut BTreeSet::new(), 0)?;
        }
        Ok(objects)
    }
}

impl Capture<'_> {
    fn project(&mut self, object: &mut HistoryObject) -> HistoryResult<()> {
        if object.kind == HistoryKind::Tree {
            let (rows, metadata) = projection_budget(&object.bytes, self.width)?;
            if metadata <= 4 * 655360 {
            } else {
                return Err(bounds("shared listing allocation"));
            }
            charge(&mut self.rows, rows, 65536)?;
            charge(&mut self.metadata, metadata, META)?;
            object.entries = self
                .git
                .listing(&object.oid, 4 * 655360)
                .map_err(|e| malformed(format!("{e:?}")))?
                .into_iter()
                .map(|e| HistoryEntry {
                    mode: e.mode,
                    name: e.raw,
                    oid: e.oid,
                })
                .collect();
            if object.entries.len() == rows {
            } else {
                return Err(malformed("projection cardinality"));
            }
        }
        Ok(())
    }
}

/// Scan only bounded structural wire spans before Reader allocates projection names.
/// Validate all paths, including a cached subtree reached at a deeper position.
fn height(
    id: &str,
    objects: &BTreeMap<String, HistoryObject>,
    heights: &mut BTreeMap<String, usize>,
    path: &mut BTreeSet<String>,
    depth: usize,
) -> HistoryResult<usize> {
    let object = objects
        .get(id)
        .ok_or_else(|| malformed("missing closure"))?;
    if object.kind == HistoryKind::Blob {
        return Ok(0);
    }
    if depth <= 128 {
    } else {
        return Err(bounds("tree depth"));
    }
    if let Some(h) = heights.get(id) {
        if depth.checked_add(*h).is_some_and(|n| n <= 128) {
            return Ok(*h);
        }
        return Err(bounds("tree depth"));
    }
    if path.insert(id.into()) {
    } else {
        return Err(malformed("tree cycle"));
    }
    let mut h = 0;
    for row in &object.entries {
        if row.mode == "160000" {
            continue;
        }
        let child = height(&row.oid, objects, heights, path, depth + 1)?;
        if objects
            .get(&row.oid)
            .is_some_and(|o| o.kind == HistoryKind::Tree)
        {
            h = h.max(child.checked_add(1).ok_or_else(|| bounds("tree height"))?);
        }
    }
    path.remove(id);
    heights.insert(id.into(), h);
    Ok(h)
}

pub struct GitBoardHistorySource {
    repo: PathBuf,
    remote: String,
}
impl GitBoardHistorySource {
    pub fn new(repo: PathBuf, remote: String) -> Self {
        Self { repo, remote }
    }
}
impl BoardHistorySource for GitBoardHistorySource {
    fn capture(&self) -> HistoryResult<LinearHistory> {
        if (1..=200).contains(&self.remote.len()) {
        } else {
            return Err(bounds("source remote name"));
        }
        let git = git(&self.repo)?;
        for key in ["extensions.partialclone", "remote\\..*\\.promisor"] {
            let result = git
                .local(&["config", "--get-regexp", key], None, 65536)
                .map_err(malformed)?;
            match result.status.code() {
                Some(1) => {}
                Some(0) => return Err(HistoryError::Unsupported("promisor history".into())),
                _ => return Err(malformed("source configuration")),
            }
        }
        if git
            .text(&["rev-parse", "--is-shallow-repository"], None)
            .map_err(malformed)?
            == "false"
        {
        } else {
            return Err(HistoryError::Unsupported("shallow history".into()));
        }
        let (algorithm, width) = match git
            .text(&["rev-parse", "--show-object-format"], None)
            .map_err(malformed)?
            .as_str()
        {
            "sha1" => (HistoryAlgorithm::Sha1, 40),
            "sha256" => (HistoryAlgorithm::Sha256, 64),
            _ => return Err(HistoryError::Unsupported("object format".into())),
        };
        // Only this call resolves the source ref. Everything below follows IDs.
        let head = git
            .text(&["rev-parse", "--verify", BOARD], None)
            .map_err(malformed)?;
        oid(&head, width)?;
        let mut capture = Capture {
            git: &git,
            width,
            raw: 0,
            metadata: 0,
            rows: 0,
        };
        let mut commits = Vec::new();
        let mut seen = BTreeSet::new();
        let mut next = Some(head.clone());
        let mut pending = BTreeSet::new();
        while let Some(id) = next {
            if commits.len() < 4096 {
            } else {
                return Err(bounds("commit count"));
            }
            if seen.insert(id.clone()) {
            } else {
                return Err(malformed("commit cycle"));
            }
            let mut batch = capture.read(&[id], Some(HistoryKind::Commit))?;
            let original = batch.pop().ok_or_else(|| malformed("missing commit"))?;
            let (root, parent) = commit_links(&original.bytes, width)?;
            next = parent;
            pending.insert(root.clone());
            commits.push((root, original));
        }
        let objects = capture.closure(pending)?;
        commits.reverse();
        let revisions = commits
            .into_iter()
            .enumerate()
            .map(|(i, (root, original))| HistoryRevision {
                sequence: i as u64 + 1,
                parent: if i == 0 { None } else { Some(i as u64) },
                root,
                writer: crate::domain::project_board::entities::claim::Identity {
                    name: "Imported Git".into(),
                    email: "history@quecto.invalid".into(),
                },
                message: String::new(),
                at: 0,
                original: Some(original),
            })
            .collect();
        Ok(LinearHistory {
            algorithm,
            source_ref: BOARD.into(),
            source_remote: self.remote.clone(),
            source_head: head,
            revisions,
            objects: objects.into_values().collect(),
        })
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
